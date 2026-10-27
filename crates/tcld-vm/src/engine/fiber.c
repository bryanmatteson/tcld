/* Emscripten continuation; the native build uses corosensei with guard pages. */
#include <stdlib.h>
#include <emscripten/fiber.h>
#define STACK_BYTES (8 * 1024 * 1024)
#define ASYNC_BYTES (2 * 1024 * 1024)
typedef struct Fiber {
    emscripten_fiber_t caller, callee;
    void *stack, *caller_async, *callee_async;
    void (*entry)(void *);
    void *data;
} Fiber;
void tcld_fiber_yield(Fiber *f) { emscripten_fiber_swap(&f->callee, &f->caller); }
static void start(void *p) {
    Fiber *f = p; f->entry(f->data); tcld_fiber_yield(f);
    abort(); /* Completed continuations must never be resumed. */
}
void *tcld_fiber_new(void (*entry)(void *), void *data) {
    Fiber *f = calloc(1, sizeof(Fiber)); if (!f) return NULL;
    f->entry = entry; f->data = data;
    f->stack = malloc(STACK_BYTES);
    f->caller_async = malloc(ASYNC_BYTES); f->callee_async = malloc(ASYNC_BYTES);
    if (!f->stack || !f->caller_async || !f->callee_async) {
        free(f->stack); free(f->caller_async); free(f->callee_async); free(f); return NULL;
    }
    emscripten_fiber_init_from_current_context(&f->caller, f->caller_async, ASYNC_BYTES);
    emscripten_fiber_init(&f->callee, start, f, f->stack, STACK_BYTES, f->callee_async, ASYNC_BYTES);
    return f;
}
void tcld_fiber_resume(Fiber *f) { emscripten_fiber_swap(&f->caller, &f->callee); }
void tcld_fiber_free(Fiber *f) {
    free(f->stack); free(f->caller_async); free(f->callee_async); free(f);
}
