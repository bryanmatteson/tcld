/* Use Tcl's extension execution hooks, not a second trace implementation. */
#include "tclInt.h"
#include <stdlib.h>
typedef struct {
    Tcl_Interp *interp;
    Command *command;
    Tcl_Obj *text, *namespace;
    Tcl_Size epoch;
    int unresolved, leave_scheduled;
    Tcl_Size objc;
    Tcl_Obj **objv;
    int level, depth;
} MappedCommand;
void *tcld_trace_command_new(Tcl_Interp *interp, const char *ns,
        int objc, Tcl_Obj *const objv[], int level, int depth) {
    Tcl_Namespace *namespace = Tcl_FindNamespace(interp, ns, NULL, 0);
    Command *command = (Command *)Tcl_FindCommand(interp, Tcl_GetString(objv[0]), namespace, TCL_LEAVE_ERR_MSG);
    if (!command) return NULL;
    MappedCommand *c = calloc(1, sizeof(*c));
    if (!c) {Tcl_SetObjResult(interp,Tcl_NewStringObj("cannot allocate trace context",-1));return NULL;}
    c->interp=interp;c->command=command;c->level=level;c->depth=depth;c->objc=objc;
    c->namespace=Tcl_NewStringObj(ns,-1);Tcl_IncrRefCount(c->namespace);c->epoch=command->cmdEpoch;
    c->text=Tcl_NewListObj(objc,objv);Tcl_IncrRefCount(c->text);
    Tcl_ListObjGetElements(interp,c->text,&c->objc,&c->objv);
    command->refCount++;
    return c;
}
int tcld_trace_command_hook(void *handle, int leave, int code) {
    MappedCommand *c=handle;
    if(leave && (c->unresolved || (c->command->flags & CMD_DYING)))return TCL_OK;
    Interp *i=(Interp *)c->interp;
    Tcl_Size savedDepth=i->numLevels, savedLevel=i->varFramePtr->level;
    i->numLevels=c->depth;i->varFramePtr->level=c->level;
    Tcl_Size length;const char *text=Tcl_GetStringFromObj(c->text,&length);
    if (!leave) c->leave_scheduled = i->tracePtr || (c->command->flags & CMD_HAS_EXEC_TRACES);
    if (leave && !c->leave_scheduled) {
        i->numLevels=savedDepth;i->varFramePtr->level=savedLevel;return TCL_OK;
    }
    int result=TCL_OK,flags=leave?TCL_TRACE_LEAVE_EXEC:TCL_TRACE_ENTER_EXEC;
    if (!leave && i->tracePtr) result=TclCheckInterpTraces(c->interp,text,length,c->command,code,flags,c->objc,c->objv);
    if (!(c->command->flags & CMD_DYING) && result==TCL_OK && (c->command->flags & CMD_HAS_EXEC_TRACES))
        result=TclCheckExecutionTraces(c->interp,text,length,c->command,code,flags,c->objc,c->objv);
    if (leave && i->tracePtr && result==TCL_OK) result=TclCheckInterpTraces(c->interp,text,length,c->command,code,flags,c->objc,c->objv);
    i->numLevels=savedDepth;i->varFramePtr->level=savedLevel;
    return result;
}
void tcld_trace_command_free(void *handle) {
    MappedCommand *c=handle;
    Tcl_DecrRefCount(c->text);
    Tcl_DecrRefCount(c->namespace);
    TclCleanupCommandMacro(c->command);
    free(c);
}

/* Only mapped boundaries deliver execution events. Calls made to implement a VM
 * operation must not trace the implementation a second time. Keep Tcl's own
 * recursion flags intact while temporarily masking these host calls. */
typedef struct Watched { Command *command; struct Watched *next; } Watched;
typedef struct { Watched *head; void *muted; unsigned serial; } WatchSet;
static void free_watch(void *data, Tcl_Interp *interp) {
    WatchSet *set=data;(void)interp;
    while(set->head) {Watched *w=set->head;set->head=w->next;TclCleanupCommandMacro(w->command);free(w);}free(set);
}
static WatchSet *watch_set(Tcl_Interp *interp) {
    WatchSet *set=Tcl_GetAssocData(interp,"tcld.execution",NULL);
    if(!set) {set=calloc(1,sizeof(*set));Tcl_SetAssocData(interp,"tcld.execution",free_watch,set);}return set;
}
void tcld_trace_watch(Tcl_Interp *interp,const char *name) {
    Command *command=(Command *)Tcl_FindCommand(interp,name,NULL,TCL_GLOBAL_ONLY);
    if(!command)return;
    WatchSet *set=watch_set(interp);
    for(Watched *w=set->head;w;w=w->next)if(w->command==command)return;
    Watched *w=malloc(sizeof(*w));w->command=command;command->refCount++;w->next=set->head;set->head=w;
}
typedef struct {Command *command;int flag;} Mask;
typedef struct Mute {struct Mute *parent;Interp *interp;int flag;size_t count;unsigned owner;WatchSet *set;Mask *masks;} Mute;
static void unmask(Mute *m) {
    m->interp->flags=(m->interp->flags & ~INTERP_TRACE_IN_PROGRESS)|m->flag;
    for(size_t n=0;n<m->count;n++) m->masks[n].command->flags |= m->masks[n].flag;
}
static void mask(Mute *m) {
    m->interp->flags |= INTERP_TRACE_IN_PROGRESS;
    for(size_t n=0;n<m->count;n++) m->masks[n].command->flags &= ~CMD_HAS_EXEC_TRACES;
}
void *tcld_trace_mute(Tcl_Interp *interp) {
    Mute *m=calloc(1,sizeof(*m));m->set=watch_set(interp);m->parent=m->set->muted;m->interp=(Interp *)interp;m->flag=m->interp->flags & INTERP_TRACE_IN_PROGRESS;
    WatchSet *set=watch_set(interp);for(Watched *w=set->head;w;w=w->next)m->count++;
    m->masks=calloc(m->count,sizeof(Mask));size_t n=0;
    for(Watched *w=set->head;w;w=w->next){m->masks[n].command=w->command;m->masks[n++].flag=w->command->flags & CMD_HAS_EXEC_TRACES;}
    mask(m);m->set->muted=m;return m;
}
void tcld_trace_unmute(void *handle) {
    Mute *m=handle;if(!m->owner)unmask(m);m->set->muted=m->parent;free(m->masks);free(m);
}
static unsigned suspend_masks(WatchSet *set) {
    unsigned ticket=++set->serial;
    for(Mute *m=set->muted;m;m=m->parent)if(!m->owner){unmask(m);m->owner=ticket;}
    return ticket;
}
static void resume_masks(Mute *m,unsigned ticket) {
    if(!m)return;resume_masks(m->parent,ticket);
    if(m->owner==ticket){mask(m);m->owner=0;}
}
int tcld_trace_mapped_hook(void *handle,int leave,int code) {
    WatchSet *set=watch_set(((MappedCommand *)handle)->interp);
    unsigned ticket=suspend_masks(set);
    int result=tcld_trace_command_hook(handle,leave,code);
    resume_masks(set->muted,ticket);return result;
}

/* Ensemble rewriting is itself a Tcl command boundary (e.g. namespace eval
 * becomes ::tcl::namespace::eval). Resolve its real, current Tcl mapping. */
void *tcld_trace_ensemble(void *handle) {
    MappedCommand *c=handle;
    if(c->objc<2)return NULL;
    Tcl_Command token=(Tcl_Command)c->command;
    if(!strcmp(Tcl_GetString(c->objv[0]),"namespace") || !strcmp(Tcl_GetString(c->objv[0]),"::namespace"))
        token=Tcl_FindCommand(c->interp,"::tcld::_namespace_orig",NULL,TCL_GLOBAL_ONLY);
    if(!token || !Tcl_IsEnsemble(token))return NULL;
    Tcl_InterpState state=Tcl_SaveInterpState(c->interp,TCL_OK);
    Tcl_Obj *mapping=NULL,*prefix=NULL;
    Tcl_GetEnsembleMappingDict(c->interp,token,&mapping);
    if(mapping) {
        Tcl_DictObjGet(c->interp,mapping,c->objv[1],&prefix);
        int flags=0;Tcl_GetEnsembleFlags(c->interp,token,&flags);
        if(!prefix && (flags & TCL_ENSEMBLE_PREFIX)) {
            Tcl_DictSearch search;Tcl_Obj *key,*value;int done;size_t n=strlen(Tcl_GetString(c->objv[1]));
            if(Tcl_DictObjFirst(c->interp,mapping,&search,&key,&value,&done)==TCL_OK) {
                for(;!done;Tcl_DictObjNext(&search,&key,&value,&done))if(!strncmp(Tcl_GetString(key),Tcl_GetString(c->objv[1]),n)) {
                    if(prefix){prefix=NULL;break;}prefix=value;
                }
                Tcl_DictObjDone(&search);
            }
        }
    }
    MappedCommand *child=NULL;
    if(prefix) {
        Tcl_Obj *args=Tcl_DuplicateObj(prefix);Tcl_IncrRefCount(args);
        for(Tcl_Size n=2;n<c->objc;n++)Tcl_ListObjAppendElement(c->interp,args,c->objv[n]);
        Tcl_Size count;Tcl_Obj **words;
        if(Tcl_ListObjGetElements(c->interp,args,&count,&words)==TCL_OK && count>0)
            child=tcld_trace_command_new(c->interp,"::",count,words,c->level,c->depth+1);
        Tcl_DecrRefCount(args);
    }
    Tcl_RestoreInterpState(c->interp,state);return child;
}
unsigned tcld_trace_visitor_begin(Tcl_Interp *interp) {return suspend_masks(watch_set(interp));}
void tcld_trace_visitor_end(Tcl_Interp *interp,unsigned ticket) {WatchSet *set=watch_set(interp);resume_masks(set->muted,ticket);}

/* Tcl re-resolves a command modified by an enter callback without running enter
 * again. Completion traces belong to the resolved command, if one remains. */
int tcld_trace_refresh(void *handle) {
    MappedCommand *c=handle;
    if(c->epoch == c->command->cmdEpoch)return 0;
    Tcl_Namespace *ns=Tcl_FindNamespace(c->interp,Tcl_GetString(c->namespace),NULL,0);
    Command *next=ns?(Command *)Tcl_FindCommand(c->interp,Tcl_GetString(c->objv[0]),ns,0):NULL;
    if(!next){c->unresolved=1;return -1;}
    next->refCount++;TclCleanupCommandMacro(c->command);c->command=next;c->epoch=next->cmdEpoch;return 1;
}

/* A native coroutine rewind runs pending leave hooks to remove step traces,
 * while Tcl's readiness check suppresses their visitor callback evaluation. */
void tcld_trace_cancel(void *handle) {
    MappedCommand *c=handle; Interp *i=(Interp *)c->interp;
    Tcl_InterpState state=Tcl_SaveInterpState(c->interp,TCL_OK);
    int rewind=i->execEnvPtr->rewind;i->execEnvPtr->rewind=1;
    (void)tcld_trace_mapped_hook(handle,1,TCL_ERROR);
    i->execEnvPtr->rewind=rewind;
    Tcl_RestoreInterpState(c->interp,state);
}
