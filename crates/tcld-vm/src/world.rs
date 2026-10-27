//! Global and namespace state for vm-strict. Hybrid mode keeps namespace
//! variables and namespaces in embedded Tcl; vm-strict has no host, so the VM
//! keeps them here, keyed by fully qualified name.

use std::collections::{HashMap, HashSet};

use crate::error::VmError;
use crate::value::{ByteString, VmObj};
use crate::var::{VarSlot, VarTable};

#[derive(Debug)]
pub struct World {
    variables: VarTable,
    namespaces: HashSet<ByteString>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            variables: VarTable::new(),
            namespaces: HashSet::from([ByteString::from("::")]),
        }
    }
}

/// A qualified name in canonical form, as Tcl resolves it: any run of two or
/// more colons separates components, and empty components vanish.
pub fn canonical(name: &ByteString) -> ByteString {
    let bytes = name.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b':' && bytes.get(i + 1) == Some(&b':') {
            parts.push(&bytes[start..i]);
            while bytes.get(i) == Some(&b':') {
                i += 1;
            }
            start = i;
        } else {
            i += 1;
        }
    }
    parts.push(&bytes[start..]);
    let mut out = Vec::with_capacity(bytes.len() + 2);
    for part in parts.into_iter().filter(|part| !part.is_empty()) {
        out.extend_from_slice(b"::");
        out.extend_from_slice(part);
    }
    if out.is_empty() {
        out.extend_from_slice(b"::");
    }
    ByteString::from_bytes(out)
}

/// A variable may have an empty tail (`::ns::`), unlike a namespace name.
/// Keep that final separator so its storage and parent remain distinct.
fn variable_key(name: &ByteString) -> ByteString {
    let key = canonical(name);
    if name.as_bytes().ends_with(b"::") && key.as_bytes() != b"::" {
        let mut bytes = key.as_bytes().to_vec();
        bytes.extend_from_slice(b"::");
        ByteString::from_bytes(bytes)
    } else {
        key
    }
}

fn variable_error(
    operation: &str,
    name1: &ByteString,
    name2: Option<&ByteString>,
    reason: &str,
) -> VmError {
    let name = match name2 {
        Some(key) => format!("{}({})", name1.as_str_lossy(), key.as_str_lossy()),
        None => name1.as_str_lossy().into_owned(),
    };
    VmError::TclError {
        message: format!("can't {operation} \"{name}\": {reason}"),
        error_info: None,
        error_code: None,
    }
}

/// `error` with Tcl's error code.
fn with_code(error: VmError, code: &[&str]) -> VmError {
    match error {
        VmError::TclError { message, .. } => crate::args::tcl_error(message, code),
        other => other,
    }
}

impl World {
    pub fn get(&self, name1: &ByteString, name2: Option<&ByteString>) -> Result<VmObj, VmError> {
        let missing = || variable_error("read", name1, name2, "no such variable");
        match (self.variables.get_slot(&variable_key(name1)), name2) {
            (Some(VarSlot::Scalar(value)), None) => Ok(value.clone()),
            (Some(VarSlot::Scalar(_)), Some(_)) => {
                Err(variable_error("read", name1, name2, "variable isn't array"))
            }
            (Some(VarSlot::Array(_)), None) => {
                Err(variable_error("read", name1, name2, "variable is array"))
            }
            (Some(VarSlot::Array(items)), Some(key)) => items
                .get(key)
                .cloned()
                .ok_or_else(|| variable_error("read", name1, name2, "no such element in array")),
            _ => Err(missing()),
        }
    }

    pub fn set(
        &mut self,
        name1: &ByteString,
        name2: Option<&ByteString>,
        value: VmObj,
    ) -> Result<(), VmError> {
        let key = variable_key(name1);
        let parent_end = key
            .as_bytes()
            .windows(2)
            .rposition(|pair| pair == b"::")
            .unwrap_or(0);
        let parent = if parent_end == 0 {
            ByteString::from("::")
        } else {
            ByteString::from_bytes(key.as_bytes()[..parent_end].to_vec())
        };
        if !self.namespace_exists(&parent) {
            return Err(with_code(
                variable_error("set", name1, name2, "parent namespace doesn't exist"),
                &["TCL", "LOOKUP", "VARNAME", &name1.as_str_lossy()],
            ));
        }
        match (self.variables.get_slot_mut(&key), name2) {
            (Some(VarSlot::Array(_)), None) => Err(with_code(
                variable_error("set", name1, name2, "variable is array"),
                &["TCL", "WRITE", "VARNAME"],
            )),
            (Some(VarSlot::Array(items)), Some(element)) => {
                items.insert(element.clone(), value);
                Ok(())
            }
            (Some(VarSlot::Scalar(_)), Some(_)) => Err(with_code(
                variable_error("set", name1, name2, "variable isn't array"),
                &["TCL", "LOOKUP", "VARNAME", &name1.as_str_lossy()],
            )),
            (_, Some(element)) => {
                self.variables.store_array_elem(key, element.clone(), value);
                Ok(())
            }
            (_, None) => {
                self.variables.store_scalar(key, value);
                Ok(())
            }
        }
    }

    pub fn unset(&mut self, name1: &ByteString, name2: Option<&ByteString>) -> Result<(), VmError> {
        let key = variable_key(name1);
        match (self.variables.get_slot_mut(&key), name2) {
            (Some(VarSlot::Array(items)), Some(element)) => items
                .remove(element)
                .map(|_| ())
                .ok_or_else(|| variable_error("unset", name1, name2, "no such element in array")),
            (Some(VarSlot::Scalar(_)), Some(_)) => Err(variable_error(
                "unset",
                name1,
                name2,
                "variable isn't array",
            )),
            (Some(VarSlot::Scalar(_) | VarSlot::Array(_)), None) => {
                self.variables.remove(&key);
                Ok(())
            }
            _ => Err(variable_error("unset", name1, name2, "no such variable")),
        }
    }

    pub fn array_exists(&self, name1: &ByteString) -> bool {
        matches!(
            self.variables.get_slot(&variable_key(name1)),
            Some(VarSlot::Array(_))
        )
    }

    /// The `array get` list of an array, or the empty list.
    pub fn array_get(&self, name1: &ByteString) -> VmObj {
        match self.variables.get_slot(&variable_key(name1)) {
            Some(VarSlot::Array(items)) => VmObj::List(
                items
                    .iter()
                    .flat_map(|(key, value)| [VmObj::Bytes(key.clone()), value.clone()])
                    .collect(),
            ),
            _ => VmObj::List(Vec::new()),
        }
    }

    /// Replace a variable with an array holding `items`.
    pub fn replace_array(&mut self, name1: &ByteString, items: HashMap<ByteString, VmObj>) {
        self.variables
            .set_slot(variable_key(name1), VarSlot::Array(items));
    }

    /// Create a namespace and its parents.
    pub fn create_namespace(&mut self, namespace: &ByteString) {
        let canonical = canonical(namespace);
        let bytes = canonical.as_bytes();
        let mut end = 2;
        while let Some(offset) = bytes[end..].windows(2).position(|pair| pair == b"::") {
            self.namespaces
                .insert(ByteString::from_bytes(bytes[..end + offset].to_vec()));
            end += offset + 2;
        }
        self.namespaces.insert(canonical);
    }

    pub fn namespace_exists(&self, namespace: &ByteString) -> bool {
        self.namespaces.contains(&canonical(namespace))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> ByteString {
        ByteString::from(text)
    }

    #[test]
    fn canonical_names_collapse_separators() {
        for (raw, expected) in [
            ("::", "::"),
            ("::x", "::x"),
            ("::a::::b", "::a::b"),
            ("::a::b::", "::a::b"),
            ("a::b", "::a::b"),
        ] {
            assert_eq!(canonical(&name(raw)), name(expected), "{raw}");
        }
    }

    #[test]
    fn variable_parents_must_exist_and_empty_tails_have_separate_storage() {
        let mut world = World::default();
        match world.set(&name("::missing::x"), None, VmObj::Int(1)) {
            Err(VmError::TclError {
                message,
                error_code,
                ..
            }) => {
                assert_eq!(
                    message,
                    "can't set \"::missing::x\": parent namespace doesn't exist"
                );
                assert_eq!(
                    error_code.as_deref(),
                    Some("TCL LOOKUP VARNAME ::missing::x")
                );
            }
            other => panic!("{other:?}"),
        }
        world.create_namespace(&name("::n"));
        world.set(&name("::n::"), None, VmObj::Int(1)).unwrap();
        world.set(&name("::n"), None, VmObj::Int(2)).unwrap();
        assert_eq!(world.get(&name("::n::::"), None).unwrap(), VmObj::Int(1));
        assert_eq!(world.get(&name("::n"), None).unwrap(), VmObj::Int(2));
        world.unset(&name("::n::"), None).unwrap();
        assert!(world.get(&name("::n::"), None).is_err());
        assert_eq!(world.get(&name("::n"), None).unwrap(), VmObj::Int(2));
        assert!(world.namespace_exists(&name("::n")));
    }

    #[test]
    fn variables_and_namespaces() {
        let mut world = World::default();
        world.set(&name("::x"), None, VmObj::Int(1)).unwrap();
        world
            .set(&name("::a"), Some(&name("k")), VmObj::Int(2))
            .unwrap();
        assert_eq!(world.get(&name("::::x"), None).unwrap(), VmObj::Int(1));
        assert!(world.get(&name("::a"), None).is_err());
        assert!(
            world
                .set(&name("::x"), Some(&name("k")), VmObj::Int(3))
                .is_err()
        );
        assert!(world.array_exists(&name("::a")));
        world.unset(&name("::x"), None).unwrap();
        assert!(world.unset(&name("::x"), None).is_err());
        world.create_namespace(&name("::p::q"));
        assert!(world.namespace_exists(&name("::p")));
        assert!(world.namespace_exists(&name("::p::q::")));
        assert!(!world.namespace_exists(&name("::q")));
    }
}
