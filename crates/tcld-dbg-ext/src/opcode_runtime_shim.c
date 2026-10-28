#include <stddef.h>
#include <stdint.h>
#include <tcl.h>

#if TCL_MAJOR_VERSION < 9
typedef int Tcld_Size;
#else
typedef Tcl_Size Tcld_Size;
#endif

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <dlfcn.h>
#include <pthread.h>
#endif

typedef int (*TcldOpcodeHook)(
    void *clientData,
    Tcl_Interp *interp,
    Tcld_Size pcOffset,
    unsigned int bcFlags,
    Tcld_Size numLevels);

typedef int (*tcld_opcode_supported_fn)(Tcl_Interp *interp);
typedef int (*tcld_opcode_set_hook_fn)(Tcl_Interp *interp, TcldOpcodeHook hook, void *clientData);
typedef void (*tcld_opcode_arm_fn)(Tcl_Interp *interp, int on);
typedef int (*tcld_opcode_current_pc_fn)(Tcl_Interp *interp, Tcld_Size *pc_out);
typedef int (*tcld_opcode_current_source_fn)(
    Tcl_Interp *interp,
    const char **file_out,
    Tcld_Size *line_out);

#if defined(_WIN32)
static INIT_ONCE g_resolve_once = INIT_ONCE_STATIC_INIT;
#else
static pthread_once_t g_resolve_once = PTHREAD_ONCE_INIT;
#endif
static tcld_opcode_supported_fn g_supported = NULL;
static tcld_opcode_set_hook_fn g_set_hook = NULL;
static tcld_opcode_arm_fn g_arm = NULL;
static tcld_opcode_current_pc_fn g_current_pc = NULL;
static tcld_opcode_current_source_fn g_current_source = NULL;

static void *resolve_symbol(const char *name) {
#if defined(_WIN32)
    HMODULE module = GetModuleHandleA(NULL);
    if (!module) {
        return NULL;
    }
    return (void *) GetProcAddress(module, name);
#else
    void *handle = dlopen(NULL, RTLD_NOW);
    if (!handle) {
        return NULL;
    }
    return dlsym(handle, name);
#endif
}

static void resolve_symbols(void) {
    g_supported = (tcld_opcode_supported_fn) resolve_symbol("tcld_opcode_supported");
    g_set_hook = (tcld_opcode_set_hook_fn) resolve_symbol("tcld_opcode_set_hook");
    g_arm = (tcld_opcode_arm_fn) resolve_symbol("tcld_opcode_arm");
    g_current_pc = (tcld_opcode_current_pc_fn) resolve_symbol("tcld_opcode_current_pc");
    g_current_source =
        (tcld_opcode_current_source_fn) resolve_symbol("tcld_opcode_current_source");
}

#if defined(_WIN32)
static BOOL CALLBACK resolve_symbols_callback(
    PINIT_ONCE once, PVOID parameter, PVOID *context) {
    (void) once;
    (void) parameter;
    (void) context;
    resolve_symbols();
    return TRUE;
}
#endif

static void resolve_symbols_once(void) {
#if defined(_WIN32)
    InitOnceExecuteOnce(&g_resolve_once, resolve_symbols_callback, NULL, NULL);
#else
    pthread_once(&g_resolve_once, resolve_symbols);
#endif
}

int tcld_opcode_runtime_supported(Tcl_Interp *interp) {
    resolve_symbols_once();
    if (!g_supported || !g_set_hook || !g_arm || !g_current_pc || !g_current_source) {
        return 0;
    }
    return g_supported(interp) ? 1 : 0;
}

int tcld_opcode_runtime_set_hook(Tcl_Interp *interp, TcldOpcodeHook hook, void *clientData) {
    resolve_symbols_once();
    if (!g_set_hook) {
        return -1;
    }
    return g_set_hook(interp, hook, clientData);
}

int tcld_opcode_runtime_arm(Tcl_Interp *interp, int on) {
    resolve_symbols_once();
    if (!g_arm) {
        return -1;
    }
    g_arm(interp, on);
    return 0;
}

int tcld_opcode_runtime_current_pc(Tcl_Interp *interp, Tcld_Size *pc_out) {
    resolve_symbols_once();
    if (!g_current_pc) {
        return -1;
    }
    return g_current_pc(interp, pc_out);
}

int tcld_opcode_runtime_current_source(
    Tcl_Interp *interp,
    const char **file_out,
    Tcld_Size *line_out) {
    resolve_symbols_once();
    if (!g_current_source) {
        return -1;
    }
    return g_current_source(interp, file_out, line_out);
}
