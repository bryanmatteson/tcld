//! Stackful debugger continuation. Pausing inside a real Tcl trace preserves
//! the actual Tcl operation, its trace cursor, errors and recursion guards.
use super::*;
use std::ffi::c_void;
#[cfg(target_os = "emscripten")]
unsafe extern "C" {
    fn tcld_fiber_new(entry: extern "C" fn(*mut c_void), data: *mut c_void) -> *mut c_void;
    fn tcld_fiber_resume(fiber: *mut c_void);
    fn tcld_fiber_yield(fiber: *mut c_void);
    fn tcld_fiber_free(fiber: *mut c_void);
}
#[cfg(not(target_os = "emscripten"))]
struct NativeFiber {
    coroutine: Option<corosensei::Coroutine<(), (), ()>>,
    yielder: *const corosensei::Yielder<(), ()>,
}
#[cfg(not(target_os = "emscripten"))]
unsafe fn tcld_fiber_new(entry: extern "C" fn(*mut c_void), data: *mut c_void) -> *mut c_void {
    let Ok(stack) = corosensei::stack::DefaultStack::new(8 * 1024 * 1024) else {
        return std::ptr::null_mut();
    };
    let fiber = Box::into_raw(Box::new(NativeFiber {
        coroutine: None,
        yielder: std::ptr::null(),
    }));
    let coroutine = corosensei::Coroutine::with_stack(stack, move |yielder, ()| {
        unsafe { (*fiber).yielder = yielder };
        entry(data);
    });
    unsafe { (*fiber).coroutine = Some(coroutine) };
    fiber.cast()
}
#[cfg(not(target_os = "emscripten"))]
unsafe fn tcld_fiber_resume(fiber: *mut c_void) {
    let fiber = fiber.cast::<NativeFiber>();
    let mut coroutine = unsafe { (*fiber).coroutine.take().unwrap() };
    let _ = coroutine.resume(());
    unsafe { (*fiber).coroutine = Some(coroutine) };
}
#[cfg(not(target_os = "emscripten"))]
unsafe fn tcld_fiber_yield(fiber: *mut c_void) {
    unsafe { (&*(*fiber.cast::<NativeFiber>()).yielder).suspend(()) };
}
#[cfg(not(target_os = "emscripten"))]
unsafe fn tcld_fiber_free(fiber: *mut c_void) {
    drop(unsafe { Box::from_raw(fiber.cast::<NativeFiber>()) });
}
pub(super) struct Suspension {
    handle: *mut c_void,
    result: Option<Result<LoopExit, VmError>>,
    running: bool,
    pub(super) cancelling: bool,
    #[cfg(feature = "host")]
    context: Option<crate::bridge::BridgeContext>,
}
impl Drop for Suspension {
    fn drop(&mut self) {
        unsafe { tcld_fiber_free(self.handle) }
    }
}
extern "C" fn execute(data: *mut c_void) {
    let engine = unsafe { &mut *(data as *mut HybridEngineInner) };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(feature = "host")]
        if let Some(mut host) = engine.host.take() {
            let ptr = engine as *mut HybridEngineInner;
            let result = host.with_stub_dispatch(
                Box::new(move |args| unsafe { (&mut *ptr).dispatch_stub_reenter(args) }),
                |_| engine.eval_loop(0, true),
            );
            engine.host = Some(host);
            return result;
        }
        engine.eval_loop(0, true)
    }))
    .unwrap_or_else(|_| Err(VmError::Internal("panic in debugger continuation".into())));
    engine.suspension.as_mut().unwrap().result = Some(result);
}
impl HybridEngineInner {
    pub(super) fn resume_suspension(&mut self) -> Result<LoopExit, VmError> {
        if self.suspension.is_none() {
            let handle = unsafe { tcld_fiber_new(execute, self as *mut Self as *mut c_void) };
            if handle.is_null() {
                return Err(VmError::Internal(
                    "cannot allocate debugger continuation".into(),
                ));
            }
            self.suspension = Some(Suspension {
                handle,
                result: None,
                running: false,
                cancelling: false,
                #[cfg(feature = "host")]
                context: None,
            });
        }
        let suspension = self.suspension.as_mut().unwrap();
        suspension.running = true;
        let handle = suspension.handle;
        #[cfg(feature = "host")]
        let caller_context = crate::bridge::take_context();
        #[cfg(feature = "host")]
        if let Some(context) = self.suspension.as_mut().unwrap().context.take() {
            crate::bridge::put_context(context);
        }
        unsafe { tcld_fiber_resume(handle) };
        #[cfg(feature = "host")]
        {
            self.suspension.as_mut().unwrap().context = Some(crate::bridge::take_context());
            crate::bridge::put_context(caller_context);
        }
        let suspension = self.suspension.as_mut().unwrap();
        suspension.running = false;
        if let Some(result) = suspension.result.take() {
            self.suspension = None;
            result
        } else {
            Ok(LoopExit::Paused)
        }
    }
    pub(super) fn pause_suspension(&mut self) -> Result<bool, VmError> {
        let Some(s) = self.suspension.as_ref().filter(|s| s.running) else {
            return Ok(false);
        };
        let handle = s.handle;
        unsafe { tcld_fiber_yield(handle) };
        if self.suspension.as_ref().unwrap().cancelling {
            Err(VmError::Internal("debug session cancelled".into()))
        } else {
            self.debug.skip_once = false;
            Ok(true)
        }
    }
    #[cfg(feature = "host")]
    pub(super) fn with_suspended_bridge<R>(
        &self,
        f: impl FnOnce(&mut HostBridge) -> R,
    ) -> Option<R> {
        if let Some(context) = self.suspension.as_ref().and_then(|s| s.context.as_ref()) {
            crate::bridge::with_context_bridge(context, f)
        } else {
            HostBridge::with_active_bridge(f)
        }
    }
    pub(super) fn callbacks_can_pause(&self) -> bool {
        self.suspension
            .as_ref()
            .is_some_and(|s| s.running && !s.cancelling)
            && !self.debug_frame_eval_active
    }
    pub(crate) fn cancel_suspension(&mut self) {
        if let Some(s) = self.suspension.as_mut() {
            s.cancelling = true;
            let _ = self.resume_suspension();
        }
    }
}
