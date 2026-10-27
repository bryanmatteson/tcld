/* Tcl owns coroutine commands, NRE execution environments and yieldto tails.
 * The mapped VM only lends Tcl a resumable source-frame continuation. */
#include <tclInt.h>
#include <stddef.h>
extern int tcld_coroutine_dispatch(Tcl_Interp *, int, Tcl_Obj *const [], int *);
#define MAPPED_YIELD 200
static int resume(void *data[], Tcl_Interp *interp, int result);
static int tail_transfer(void *, Tcl_Interp *, int, Tcl_Obj *const []);
static int control_nr(void *, Tcl_Interp *, int, Tcl_Obj *const []);
static int drive(Tcl_Interp *interp, Tcl_Obj *id, const char *op, int code) {
    Tcl_Obj *v[8];
    v[0] = Tcl_NewStringObj("::tcld::coroutine_body", -1);
    v[1] = id; v[2] = Tcl_NewStringObj(op, -1);
    v[3] = Tcl_NewIntObj(code); v[4] = Tcl_GetObjResult(interp);
    v[5] = Tcl_GetReturnOptions(interp, code);
    CoroutineData *cor = ((Interp *)interp)->execEnvPtr->corPtr;
    v[6] = Tcl_NewWideIntObj((Tcl_WideInt)(uintptr_t)cor);
    v[7] = Tcl_NewWideIntObj((Tcl_WideInt)(uintptr_t)(cor ? cor->callerEEPtr->corPtr : NULL));
    for (int i=0;i<8;i++) Tcl_IncrRefCount(v[i]);
    /* Native proc destruction calls TclDeleteVars directly while rewinding.
     * The VM stores locals in Tcl namespaces, so admit its equivalent cleanup
     * evaluator calls; Tcl's own variable-trace code admits unset callbacks. */
    int rewind = ((Interp *)interp)->execEnvPtr->rewind;
    if (strcmp(op, "cancel") == 0) ((Interp *)interp)->execEnvPtr->rewind = 0;
    int handoff = 0;
    int rc = tcld_coroutine_dispatch(interp, 8, v, &handoff);
    ((Interp *)interp)->execEnvPtr->rewind = rewind;
    for (int i=0;i<8;i++) Tcl_DecrRefCount(v[i]);
    /* Handoff metadata is separate from the completion code: Tcl scripts may
     * legitimately return any integer code, including 200 and 201. */
    if (handoff == 201) {
        Tcl_Obj *args[3] = {Tcl_NewStringObj("::tcld::tail_transfer", -1), id, Tcl_GetObjResult(interp)};
        for (int i=0;i<3;i++) Tcl_IncrRefCount(args[i]);
        int result = tail_transfer(NULL, interp, 3, args);
        for (int i=0;i<3;i++) Tcl_DecrRefCount(args[i]);
        return result;
    }
    if (handoff != MAPPED_YIELD) return rc;
    Tcl_Obj *args = Tcl_GetObjResult(interp); Tcl_IncrRefCount(args);
    Tcl_Size n; Tcl_Obj **words;
    if (Tcl_ListObjGetElements(interp, args, &n, &words) != TCL_OK) {
        Tcl_DecrRefCount(args); return TCL_ERROR;
    }
    /* The resume callback survives on Tcl's coroutine NRE stack. */
    Tcl_IncrRefCount(id);
    Tcl_NRAddCallback(interp, resume, id, args, NULL, NULL);
    const char *name = Tcl_GetString(words[0]);
    if (strcmp(name, "::tcld::control") == 0) {
        /* An adapter call is not another visitor evaluation level. In
         * particular a lambda tail must not retain one EvalObjv per hop. */
        Tcl_ResetResult(interp);
        return control_nr(NULL,interp,(int)n,words);
    }
    if (strcmp(name, "yield") != 0 && strcmp(name, "yieldto") != 0) {
        Tcl_ResetResult(interp);
        return Tcl_NREvalObjv(interp, n, words, 0);
    }
    Command *cmd = (Command *)Tcl_FindCommand(interp,
        strcmp(name, "yieldto") == 0 ? "::tcld::_frame_yieldto_orig" : "::tcld::_frame_yield_orig",
        NULL, TCL_GLOBAL_ONLY);
    Tcl_ResetResult(interp);
    if (!cmd || !cmd->nreProc) {
        Tcl_SetObjResult(interp, Tcl_NewStringObj("native Tcl yield command unavailable", -1));
        return TCL_ERROR;
    }
    return cmd->nreProc(cmd->objClientData, interp, (int)n, words);
}
static int resume(void *data[], Tcl_Interp *interp, int result) {
    Tcl_Obj *id = data[0];
    int rc = drive(interp, id,
        ((Interp *)interp)->execEnvPtr->rewind ? "cancel" : "resume", result);
    Tcl_DecrRefCount((Tcl_Obj *)data[1]); Tcl_DecrRefCount(id);
    return rc;
}
static int nr(void *unused, Tcl_Interp *interp, int objc, Tcl_Obj *const objv[]) {
    (void)unused;
    if (objc != 2) { Tcl_WrongNumArgs(interp, 1, objv, "id"); return TCL_ERROR; }
    return drive(interp, objv[1], strcmp(Tcl_GetString(objv[0]), "::tcld::tail_body") == 0 ? "tail" : "start", TCL_OK);
}
static int obj(void *data, Tcl_Interp *interp, int objc, Tcl_Obj *const objv[]) {
    return Tcl_NRCallObjProc(interp, nr, data, objc, objv);
}
typedef struct { Tcl_Interp *interp; Tcl_Obj *list; } PendingTail;
static void free_tails(void *data, Tcl_Interp *interp) {
    (void)interp; Tcl_HashTable *table=data; Tcl_HashSearch search;
    for(Tcl_HashEntry *e=Tcl_FirstHashEntry(table,&search);e;e=Tcl_NextHashEntry(&search)) {
        PendingTail *p=Tcl_GetHashValue(e); Tcl_DecrRefCount(p->list); Tcl_Free(p);
    }
    Tcl_DeleteHashTable(table); Tcl_Free(table);
}
static Tcl_HashTable *tails(Tcl_Interp *interp) {
    Tcl_HashTable *t=Tcl_GetAssocData(interp,"tcld.pending-tails",NULL);
    if(!t) {t=(Tcl_HashTable *)Tcl_Alloc(sizeof(*t));Tcl_InitHashTable(t,TCL_ONE_WORD_KEYS);Tcl_SetAssocData(interp,"tcld.pending-tails",free_tails,t);}return t;
}
/* Preserve the real native pending-tail object, including its namespace
 * reference. Tcl prepares it now and schedules it only after mapped cleanup. */
int tcld_tailcall_prepare(Tcl_Interp *interp, const char *ns, int allowed,
        int objc, Tcl_Obj *const objv[], void **pending) {
    Tcl_CallFrame frame;
    Tcl_Namespace *namespace = Tcl_FindNamespace(interp, ns, NULL, TCL_LEAVE_ERR_MSG);
    *pending = NULL;
    if (!namespace || Tcl_PushCallFrame(interp, &frame, namespace, allowed) != TCL_OK) return TCL_ERROR;
    Command *cmd = (Command *)Tcl_FindCommand(interp, "::tailcall", NULL, TCL_GLOBAL_ONLY);
    Tcl_ResetResult(interp);
    int rc;
    if (!cmd || !cmd->nreProc) { Tcl_SetObjResult(interp,Tcl_NewStringObj("native Tcl tailcall command unavailable",-1)); rc=TCL_ERROR; }
    else rc = cmd->nreProc(cmd->objClientData, interp, objc, objv);
    CallFrame *f = (CallFrame *)&frame;
    if (f->tailcallPtr) {
        PendingTail *p=(PendingTail *)Tcl_Alloc(sizeof(*p));p->interp=interp;p->list=f->tailcallPtr;
        Tcl_IncrRefCount(p->list);f->tailcallPtr=NULL;int fresh;
        Tcl_SetHashValue(Tcl_CreateHashEntry(tails(interp),(char *)p,&fresh),p);*pending=p;
    }
    Tcl_PopCallFrame(interp);
    return rc;
}
int tcld_tailcall_validate(Tcl_Interp *interp, void *pending) {
    PendingTail *p=pending; Tcl_Obj *ns; Tcl_Namespace *namespace;
    Tcl_ListObjIndex(interp,p->list,0,&ns);
    return TclGetNamespaceFromObj(interp,ns,&namespace);
}
void tcld_tailcall_free(void *pending) {
    PendingTail *p=pending; Tcl_HashEntry *e=Tcl_FindHashEntry(tails(p->interp),(char *)p);
    if(e)Tcl_DeleteHashEntry(e);Tcl_DecrRefCount(p->list);Tcl_Free(p);
}
static int pop_tail(void *data[], Tcl_Interp *interp, int result) {
    Tcl_PopCallFrame(interp); Tcl_Free(data[0]); return result;
}
static int tail_transfer(void *unused, Tcl_Interp *interp, int objc, Tcl_Obj *const objv[]) {
    (void)unused;
    if (objc != 3) { Tcl_WrongNumArgs(interp, 1, objv, "id pending"); return TCL_ERROR; }
    Tcl_WideInt address;
    if (Tcl_GetWideIntFromObj(interp, objv[2], &address) != TCL_OK) return TCL_ERROR;
    Tcl_HashEntry *entry = Tcl_FindHashEntry(tails(interp),(char *)(uintptr_t)address);
    if (!entry) {Tcl_SetObjResult(interp,Tcl_NewStringObj("invalid native tailcall token",-1));return TCL_ERROR;}
    PendingTail *p=Tcl_GetHashValue(entry);
    Tcl_Obj *pending = p->list;
    Tcl_Obj *ns;
    Tcl_ListObjIndex(interp, pending, 0, &ns);
    Tcl_Obj *parts[3] = {ns, Tcl_NewStringObj("::tcld::tail_body", -1), objv[1]};
    Tcl_Obj *redirect = Tcl_NewListObj(3, parts); Tcl_IncrRefCount(redirect);
    tcld_tailcall_free(p);
    Tcl_CallFrame *frame = (Tcl_CallFrame *)Tcl_Alloc(sizeof(Tcl_CallFrame));
    if (Tcl_PushCallFrame(interp, frame, Tcl_GetGlobalNamespace(interp), 1) != TCL_OK) {
        Tcl_Free(frame); Tcl_DecrRefCount(redirect); return TCL_ERROR;
    }
    ((CallFrame *)frame)->tailcallPtr = redirect;
    Tcl_NRAddCallback(interp, pop_tail, frame, NULL, NULL, NULL);
    Tcl_ResetResult(interp);
    return TCL_OK;
}
static int obj_tail(void *data, Tcl_Interp *interp, int objc, Tcl_Obj *const objv[]) {
    return Tcl_NRCallObjProc(interp, tail_transfer, data, objc, objv);
}
/* Locate substitutions through Tcl's public parsers. No substitutions execute
 * here: Tcl_NRSubstObj still owns expansion and its completion semantics.
 * TclSubstParse is module-private in shared Tcl builds, so use its public
 * command/variable parsers and its valid-prefix rule on malformed commands. */
static void subst_command(Tcl_Interp *interp, Tcl_Obj *commands,
        const char *bytes, const char *start, Tcl_Size size) {
    Tcl_Obj *parts[3]={Tcl_NewWideIntObj(start-bytes),
        Tcl_NewWideIntObj(size),Tcl_NewStringObj(start+1,size-2)};
    Tcl_ListObjAppendElement(interp,commands,Tcl_NewListObj(3,parts));
}
static void subst_variable_tokens(Tcl_Interp *interp, Tcl_Parse *parse,
        const char *bytes, Tcl_Obj *vars, Tcl_Obj *commands) {
    for(Tcl_Size i=0;i<parse->numTokens;i++) {
        Tcl_Token *token=&parse->tokenPtr[i];
        if(token->type==TCL_TOKEN_VARIABLE) {
            Tcl_Token *name=token+1;
            Tcl_ListObjAppendElement(interp,vars,Tcl_NewStringObj(name->start,name->size));
        } else if(token->type==TCL_TOKEN_COMMAND) {
            subst_command(interp,commands,bytes,token->start,token->size);
        }
    }
}
Tcl_Obj *tcld_subst_plan(Tcl_Interp *interp, int objc, Tcl_Obj *const objv[]) {
    static const char *const options[]={"-nobackslashes","-nocommands","-novariables",NULL};
    static const int masks[]={TCL_SUBST_BACKSLASHES,TCL_SUBST_COMMANDS,TCL_SUBST_VARIABLES};
    int flags=TCL_SUBST_ALL;
    Tcl_InterpState saved=Tcl_SaveInterpState(interp,TCL_OK);
    if(objc<2) {Tcl_RestoreInterpState(interp,saved);return NULL;}
    for(int i=1;i<objc-1;i++) {
        int index;
        if(Tcl_GetIndexFromObj(interp,objv[i],options,"option",0,&index)!=TCL_OK) {
            Tcl_RestoreInterpState(interp,saved);return NULL;
        }
        flags &= ~masks[index];
    }
    Tcl_Size length; const char *bytes=Tcl_GetStringFromObj(objv[objc-1],&length);
    const char *p=bytes, *end=bytes+length;
    Tcl_Obj *vars=Tcl_NewListObj(0,NULL), *commands=Tcl_NewListObj(0,NULL);
    while(p<end) {
        if(*p=='\\' && (flags&TCL_SUBST_BACKSLASHES)) {
            int read; char buffer[TCL_UTF_MAX];Tcl_UtfBackslash(p,&read,buffer);
            p+=read;continue;
        }
        if(*p=='$' && (flags&TCL_SUBST_VARIABLES)) {
            Tcl_Parse parse;
            if(Tcl_ParseVarName(interp,p,end-p,&parse,0)!=TCL_OK) break;
            subst_variable_tokens(interp,&parse,bytes,vars,commands);
            p+=parse.tokenPtr[0].size;Tcl_FreeParse(&parse);continue;
        }
        if(*p=='[' && (flags&TCL_SUBST_COMMANDS)) {
            const char *start=p++, *next=p;
            int complete=0;
            while(next<end) {
                Tcl_Parse parse;
                if(Tcl_ParseCommand(interp,next,end-next,1,&parse)!=TCL_OK) break;
                next=parse.commandStart+parse.commandSize;
                complete=parse.term<end && *parse.term==']' && !parse.incomplete;
                Tcl_FreeParse(&parse);if(complete) break;
            }
            if(complete) {
                subst_command(interp,commands,bytes,start,next-start);p=next;continue;
            }
            /* Match TclSubstParse: evaluate only complete commands before the
             * error. Keep the original final terminator and malformed suffix,
             * so native subst defers the same error (or suppresses it on break).
             * The final unterminated command is deliberately not included. */
            const char *last=start;next=start+1;
            while(next<end) {
                Tcl_Parse parse;
                if(Tcl_ParseCommand(NULL,next,end-next,0,&parse)!=TCL_OK) break;
                const char *term=parse.term;
                Tcl_FreeParse(&parse);
                if(term==end) break;
                last=term;next=term+1;
            }
            if(last!=start) subst_command(interp,commands,bytes,start,last-start+1);
            break;
        }
        p++;
    }
    Tcl_Obj *parts[3]={objv[objc-1],vars,commands};
    Tcl_Obj *plan=Tcl_NewListObj(3,parts);Tcl_IncrRefCount(plan);
    Tcl_RestoreInterpState(interp,saved);
    return plan;
}
/* Keep Tcl's real control state machines on their NRE stacks. Script words
 * are mapped continuations; cleanup releases unselected branches as well. */
typedef struct ControlContext {
    struct ControlContext *previous;
    CoroutineData *owner;
    Tcl_WideInt id;
    CallFrame *variables;
    const char *dictionary;
    int resolving;
} ControlContext;
/* A coroutine can suspend with a control callback on its NRE stack. Select only
 * contexts in the currently executing native coroutine, never another task. */
Tcl_WideInt tcld_active_control(Tcl_Interp *interp) {
    ControlContext *context=Tcl_GetAssocData(interp,"tcld.active-control",NULL);
    CoroutineData *owner=((Interp *)interp)->execEnvPtr->corPtr;
    for (;context;context=context->previous) if(context->owner==owner) return context->id;
    return 0;
}
/* Native dict-with discovers keys only after its ordinary dictionary read.
 * Resolve its local variables to the mapped caller on demand, without reading
 * the dictionary twice or replacing Tcl's init/finalize algorithms. Restrict
 * resolution to the exact synthetic frame and current native coroutine. */
static int control_variable(Tcl_Interp *interp, const char *name, Tcl_Namespace *ns, int flags, Tcl_Var *var) {
    (void)ns;
    if ((flags & (TCL_GLOBAL_ONLY|TCL_NAMESPACE_ONLY)) || strstr(name,"::")) return TCL_CONTINUE;
    ControlContext *context=Tcl_GetAssocData(interp,"tcld.active-control",NULL);
    Interp *ip=(Interp *)interp;
    while(context && (context->owner!=ip->execEnvPtr->corPtr || context->variables!=ip->varFramePtr)) context=context->previous;
    if(!context || context->resolving) return TCL_CONTINUE;
    context->resolving=1;
    Tcl_InterpState saved=Tcl_SaveInterpState(interp,TCL_OK);
    Tcl_Obj *v[3]={Tcl_NewStringObj("::tcld::control_variable",-1),Tcl_NewWideIntObj(context->id),Tcl_NewStringObj(name,-1)};
    for(int i=0;i<3;i++) Tcl_IncrRefCount(v[i]);
    int handoff=0,rc=tcld_coroutine_dispatch(interp,3,v,&handoff);
    if(rc==TCL_OK && context->dictionary && strcmp(name,context->dictionary)==0) {
        const char *target=Tcl_GetString(Tcl_GetObjResult(interp));
        if(!strchr(target,'(') && !Tcl_FindNamespaceVar(interp,target,NULL,TCL_GLOBAL_ONLY|TCL_AVOID_RESOLVERS)) {
            /* Leave an absent dictionary variable absent, rather than creating
             * an undefined upvar (which changes Tcl's native error options). */
            for(int i=0;i<3;i++) Tcl_DecrRefCount(v[i]);
            context->resolving=0;Tcl_RestoreInterpState(interp,saved);return TCL_CONTINUE;
        }
    }
    if(rc==TCL_OK) rc=Tcl_UpVar(interp,"#0",Tcl_GetString(Tcl_GetObjResult(interp)),name,0);
    if(rc==TCL_OK) {
        Tcl_HashEntry *entry=Tcl_FindHashEntry(&context->variables->varTablePtr->table,v[2]);
        if(entry) *var=(Tcl_Var)((char *)entry - offsetof(VarInHash,entry));
        else {Tcl_SetObjResult(interp,Tcl_NewStringObj("missing mapped control variable",-1));rc=TCL_ERROR;}
    }
    for(int i=0;i<3;i++) Tcl_DecrRefCount(v[i]);
    context->resolving=0;
    if(rc==TCL_OK) Tcl_RestoreInterpState(interp,saved); else Tcl_DiscardInterpState(saved);
    return rc;
}
static int control_done(void *data[], Tcl_Interp *interp, int result) {
    ControlContext *context=data[2];
    ControlContext *head=Tcl_GetAssocData(interp,"tcld.active-control",NULL);
    if(head==context) Tcl_SetAssocData(interp,"tcld.active-control",NULL,context->previous);
    else for(ControlContext *p=head;p;p=p->previous) if(p->previous==context) {p->previous=context->previous;break;}
    Tcl_Free(context);
    if(data[3]) {Tcl_PopCallFrame(interp); Tcl_Free(data[3]);}
    Tcl_InterpState saved = Tcl_SaveInterpState(interp, result);
    Tcl_Obj *v[5] = {Tcl_NewStringObj("::tcld::control_cleanup", -1), data[0],
        Tcl_NewIntObj(result), Tcl_GetObjResult(interp), Tcl_GetReturnOptions(interp, result)};
    for (int i=0;i<5;i++) Tcl_IncrRefCount(v[i]);
    int handoff=0; tcld_coroutine_dispatch(interp, 5, v, &handoff);
    for (int i=0;i<5;i++) Tcl_DecrRefCount(v[i]);
    Tcl_DecrRefCount((Tcl_Obj *)data[0]); Tcl_DecrRefCount((Tcl_Obj *)data[1]);
    return Tcl_RestoreInterpState(interp, saved);
}

/* NRE form of Tcl 9.0's DictFilterCmd FILTER_SCRIPT loop. The upstream
 * command uses TclEvalObjEx recursively and cannot yield. Keep its dict search,
 * bindings, original key/value selection, boolean conversion and completions;
 * only schedule the admitted mapped body through Tcl's NRE evaluator instead.
 * This is reached solely after control_enter's exact, single-use admission.
 * key/value modes and malformed script calls retain upstream validation. */
typedef struct FilterContext {
    Tcl_DictSearch search;
    Tcl_Obj *keyVar, *valueVar, *body, *result, *key, *value;
} FilterContext;
static int filter_next(Tcl_Interp *, FilterContext *);
static int filter_free(Tcl_Interp *interp, FilterContext *f, int code) {
    Tcl_DictObjDone(&f->search);
    if(code==TCL_OK) Tcl_SetObjResult(interp,f->result);
    Tcl_DecrRefCount(f->keyVar); Tcl_DecrRefCount(f->valueVar);
    Tcl_DecrRefCount(f->body); Tcl_DecrRefCount(f->result);
    Tcl_Free(f); return code;
}
static int filter_done(void *data[], Tcl_Interp *interp, int code) {
    FilterContext *f=data[0];
    int satisfied=0, stop=0;
    if(code==TCL_OK) {
        Tcl_Obj *boolean=Tcl_GetObjResult(interp);Tcl_IncrRefCount(boolean);
        Tcl_ResetResult(interp);
        code=Tcl_GetBooleanFromObj(interp,boolean,&satisfied);
        Tcl_DecrRefCount(boolean);
        if(code==TCL_OK && satisfied) Tcl_DictObjPut(NULL,f->result,f->key,f->value);
    } else if(code==TCL_BREAK || code==TCL_CONTINUE) {
        stop=code==TCL_BREAK; code=TCL_OK; Tcl_ResetResult(interp);
    } else if(code==TCL_ERROR) {
        Tcl_AppendObjToErrorInfo(interp,Tcl_ObjPrintf(
            "\n    (\"dict filter\" script line %d)",Tcl_GetErrorLine(interp)));
    }
    Tcl_DecrRefCount(f->key); Tcl_DecrRefCount(f->value);
    if(code!=TCL_OK || stop) return filter_free(interp,f,code);
    int done;
    Tcl_DictObjNext(&f->search,&f->key,&f->value,&done);
    return done ? filter_free(interp,f,TCL_OK) : filter_next(interp,f);
}
static int filter_next(Tcl_Interp *interp, FilterContext *f) {
    Tcl_IncrRefCount(f->key); Tcl_IncrRefCount(f->value);
    const char *error=NULL;
    if(!Tcl_ObjSetVar2(interp,f->keyVar,NULL,f->key,TCL_LEAVE_ERR_MSG))
        error="\n    (\"dict filter\" filter script key variable)";
    else if(!Tcl_ObjSetVar2(interp,f->valueVar,NULL,f->value,TCL_LEAVE_ERR_MSG))
        error="\n    (\"dict filter\" filter script value variable)";
    if(error) {
        Tcl_AddErrorInfo(interp,error);
        Tcl_DecrRefCount(f->key); Tcl_DecrRefCount(f->value);
        return filter_free(interp,f,TCL_ERROR);
    }
    Tcl_NRAddCallback(interp,filter_done,f,NULL,NULL,NULL);
    return TclNREvalObjEx(interp,f->body,0,((Interp *)interp)->cmdFramePtr,4);
}
static int filter_nr(Tcl_Interp *interp, Tcl_Obj *dictionary,
        Tcl_Obj *variables, Tcl_Obj *body) {
    Tcl_Size count; Tcl_Obj **vars;
    if(Tcl_ListObjGetElements(interp,variables,&count,&vars)!=TCL_OK) return TCL_ERROR;
    if(count!=2) {
        Tcl_SetObjResult(interp,Tcl_NewStringObj("must have exactly two variable names",-1));
        Tcl_SetErrorCode(interp,"TCL","SYNTAX","dict","filter",NULL);
        return TCL_ERROR;
    }
    FilterContext *f=(FilterContext *)Tcl_Alloc(sizeof(*f));
    f->keyVar=vars[0]; f->valueVar=vars[1]; f->body=body;
    Tcl_IncrRefCount(f->keyVar); Tcl_IncrRefCount(f->valueVar); Tcl_IncrRefCount(body);
    int done,code=Tcl_DictObjFirst(interp,dictionary,&f->search,&f->key,&f->value,&done);
    if(code!=TCL_OK) {
        Tcl_DecrRefCount(f->keyVar);Tcl_DecrRefCount(f->valueVar);Tcl_DecrRefCount(body);
        Tcl_Free(f);return code;
    }
    f->result=Tcl_NewDictObj();Tcl_IncrRefCount(f->result);
    return done ? filter_free(interp,f,TCL_OK) : filter_next(interp,f);
}
static int control_nr(void *unused, Tcl_Interp *interp, int objc, Tcl_Obj *const objv[]) {
    (void)unused;
    if (objc < 5) {Tcl_WrongNumArgs(interp,1,objv,"id namespace aliases command ?arg ...?");return TCL_ERROR;}
    Tcl_Obj **check=(Tcl_Obj **)Tcl_Alloc(sizeof(*check)*objc);
    check[0]=Tcl_NewStringObj("::tcld::control_enter",-1);
    for(int i=1;i<objc;i++) check[i]=objv[i];
    for(int i=0;i<objc;i++) Tcl_IncrRefCount(check[i]);
    int handoff=0, admitted=tcld_coroutine_dispatch(interp,objc,check,&handoff);
    for(int i=0;i<objc;i++) Tcl_DecrRefCount(check[i]);
    Tcl_Free(check); if(admitted!=TCL_OK) return admitted;
    Tcl_WideInt id;
    Tcl_Size alias_count; Tcl_Obj **aliases;
    if(Tcl_GetWideIntFromObj(interp,objv[1],&id)!=TCL_OK || Tcl_ListObjGetElements(interp,objv[3],&alias_count,&aliases)!=TCL_OK) return TCL_ERROR;
    if(alias_count%2) {Tcl_SetObjResult(interp,Tcl_NewStringObj("invalid control aliases",-1));return TCL_ERROR;}
    int lambda=strcmp(Tcl_GetString(objv[4]),"apply")==0;
    Tcl_CallFrame *frame=NULL;
    if(!lambda) {
        Tcl_Namespace *ns=Tcl_FindNamespace(interp,Tcl_GetString(objv[2]),NULL,TCL_LEAVE_ERR_MSG);
        frame=(Tcl_CallFrame *)Tcl_Alloc(sizeof(*frame));
        if(!ns || Tcl_PushCallFrame(interp,frame,ns,1)!=TCL_OK) {Tcl_Free(frame);return TCL_ERROR;}
    }
    for(Tcl_Size i=0;i<alias_count;i+=2) {
        if(objc>=7 && strcmp(Tcl_GetString(objv[4]),"dict")==0 &&
           (strcmp(Tcl_GetString(objv[5]),"with")==0 || strcmp(Tcl_GetString(objv[5]),"update")==0) &&
           strcmp(Tcl_GetString(aliases[i]),Tcl_GetString(objv[6]))==0 &&
           !strchr(Tcl_GetString(aliases[i+1]),'(') &&
           !Tcl_FindNamespaceVar(interp,Tcl_GetString(aliases[i+1]),NULL,TCL_GLOBAL_ONLY|TCL_AVOID_RESOLVERS)) continue;
        if(Tcl_UpVar(interp,"#0",Tcl_GetString(aliases[i+1]),Tcl_GetString(aliases[i]),0)!=TCL_OK) {
            Tcl_PopCallFrame(interp);Tcl_Free(frame);return TCL_ERROR;
        }
    }
    Tcl_Obj *args=Tcl_NewListObj(objc-4,objv+4); Tcl_IncrRefCount(args);
    ControlContext *context=(ControlContext *)Tcl_Alloc(sizeof(*context));
    context->variables=objc>=8 && strcmp(Tcl_GetString(objv[4]),"dict")==0 && strcmp(Tcl_GetString(objv[5]),"with")==0 ? (CallFrame *)frame : NULL;
    context->dictionary=context->variables ? Tcl_GetString(objv[6]) : NULL;
    context->resolving=0;
    context->id=id;context->owner=((Interp *)interp)->execEnvPtr->corPtr;
    context->previous=Tcl_GetAssocData(interp,"tcld.active-control",NULL);
    Tcl_SetAssocData(interp,"tcld.active-control",NULL,context);
    Tcl_IncrRefCount(objv[1]); Tcl_NRAddCallback(interp,control_done,objv[1],args,context,frame);
    if(objc==10 && strcmp(Tcl_GetString(objv[4]),"dict")==0 &&
       strcmp(Tcl_GetString(objv[5]),"filter")==0 && strcmp(Tcl_GetString(objv[7]),"script")==0) {
        return filter_nr(interp,objv[6],objv[8],objv[9]);
    }
    if(lambda) {
        /* Apply supplies its own real procedure frame. Calling the native NRE
         * implementation directly avoids an extra EVAL/synthetic-proc frame
         * persisting after a lambda tail call and consuming Tcl eval depth. */
        Command *cmd=(Command *)Tcl_FindCommand(interp,"::apply",NULL,TCL_GLOBAL_ONLY);
        if(!cmd || !cmd->nreProc) {Tcl_SetObjResult(interp,Tcl_NewStringObj("native Tcl apply command unavailable",-1));return TCL_ERROR;}
        return cmd->nreProc(cmd->objClientData,interp,objc-4,objv+4);
    }
    /* The script evaluator supplies the real CmdFrame required by switch's
     * source-context bookkeeping. A canonical Tcl list is one safe command;
     * every visitor script argument has already become a mapped continuation.
     * Unlike Tcl_NREvalObj's INT_MIN sentinel, word 0 requests an EVAL CmdFrame
     * for canonical list objects too. Tcl owns that frame across NRE suspension. */
    return TclNREvalObjEx(interp,args,TCL_EVAL_DIRECT,NULL,0);
}
static int control_obj(void *data, Tcl_Interp *interp, int objc, Tcl_Obj *const objv[]) {
    return Tcl_NRCallObjProc(interp, control_nr, data, objc, objv);
}
void tcld_coroutine_install(Tcl_Interp *interp) {
    Tcl_AddInterpResolvers(interp,"tcld.control-variables",NULL,control_variable,NULL);
    Tcl_NRCreateCommand(interp, "::tcld::control", control_obj, control_nr, NULL, NULL);
    Tcl_NRCreateCommand(interp, "::tcld::coroutine_body", obj, nr, NULL, NULL);
    Tcl_NRCreateCommand(interp, "::tcld::coroutine_tail", obj, nr, NULL, NULL);
    Tcl_NRCreateCommand(interp, "::tcld::tail_body", obj, nr, NULL, NULL);
    Tcl_NRCreateCommand(interp, "::tcld::tail_transfer", obj_tail, tail_transfer, NULL, NULL);
}

int tcld_coroutine_is_command(Tcl_Interp *interp, const char *name, const char *namespace) {
    CoroutineData *cor = ((Interp *)interp)->execEnvPtr->corPtr;
    Tcl_Namespace *ns = Tcl_FindNamespace(interp, namespace, NULL, 0);
    Command *cmd = ns ? (Command *)Tcl_FindCommand(interp, name, ns, 0) : NULL;
    return cor && cmd && cmd->nreProc == cor->cmdPtr->nreProc;
}

/* Introspection must not execute a visitor's read traces or invent values for
 * undefined promoted locals. Namespace storage can contain arrays and links. */
int tcld_variable_defined(Tcl_Interp *interp, const char *name) {
    Var *var=(Var *)Tcl_FindNamespaceVar(interp,name,NULL,TCL_GLOBAL_ONLY|TCL_AVOID_RESOLVERS);
    while(var && TclIsVarLink(var)) var=var->value.linkPtr;
    return var && !TclIsVarUndefined(var);
}
