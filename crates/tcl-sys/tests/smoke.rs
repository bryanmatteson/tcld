use tcl_sys::*;

#[test]
fn create_interp_eval_delete() {
    unsafe {
        let interp = Tcl_CreateInterp();
        assert!(!interp.is_null());

        let ret = Tcl_Init(interp);
        assert_eq!(ret, TCL_OK, "Tcl_Init failed");

        let script = c"expr {1 + 1}";
        let ret = Tcl_EvalEx(interp, script.as_ptr(), -1, 0);
        assert_eq!(ret, TCL_OK, "eval failed");

        let result_obj = Tcl_GetObjResult(interp);
        assert!(!result_obj.is_null());
        let mut len: Tcl_Size = 0;
        let result = Tcl_GetStringFromObj(result_obj, &mut len);
        let result = std::str::from_utf8(std::slice::from_raw_parts(
            result as *const u8,
            len as usize,
        ))
        .unwrap();
        assert_eq!(result, "2");

        Tcl_DeleteInterp(interp);
    }
}
