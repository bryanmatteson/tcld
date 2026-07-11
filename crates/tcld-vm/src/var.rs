//! Variable tables and links (S03–S07).

use std::collections::HashMap;

use crate::error::VmError;
use crate::value::{ByteString, VmObj, parse_array_name};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameId(pub u32);

#[derive(Debug, Clone)]
pub enum VarLink {
    Vm {
        target_frame: FrameId,
        target_name: ByteString,
        part2: Option<ByteString>,
    },
    HostVar {
        name1: ByteString,
        name2: Option<ByteString>,
    },
}

#[derive(Debug, Clone)]
pub enum VarSlot {
    Scalar(VmObj),
    Array(HashMap<ByteString, VmObj>),
    Link(VarLink),
    Undefined,
}

#[derive(Debug, Default, Clone)]
pub struct VarTable {
    slots: HashMap<ByteString, VarSlot>,
}

impl VarTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get_slot(&self, name: &ByteString) -> Option<&VarSlot> {
        self.slots.get(name)
    }

    pub fn get_slot_mut(&mut self, name: &ByteString) -> Option<&mut VarSlot> {
        self.slots.get_mut(name)
    }

    pub fn set_slot(&mut self, name: ByteString, slot: VarSlot) {
        self.slots.insert(name, slot);
    }

    pub fn remove(&mut self, name: &ByteString) -> Option<VarSlot> {
        self.slots.remove(name)
    }

    pub fn contains(&self, name: &ByteString) -> bool {
        self.slots.contains_key(name)
    }

    /// Direct scalar store (does not follow links — caller resolves).
    pub fn store_scalar(&mut self, name: ByteString, value: VmObj) {
        self.slots.insert(name, VarSlot::Scalar(value));
    }

    pub fn store_array_elem(&mut self, name: ByteString, key: ByteString, value: VmObj) {
        match self.slots.get_mut(&name) {
            Some(VarSlot::Array(map)) => {
                map.insert(key, value);
            }
            _ => {
                let mut map = HashMap::new();
                map.insert(key, value);
                self.slots.insert(name, VarSlot::Array(map));
            }
        }
    }

    pub fn load_scalar_local(&self, name: &ByteString) -> Result<VmObj, VmError> {
        match self.slots.get(name) {
            Some(VarSlot::Scalar(v)) => Ok(v.clone()),
            Some(VarSlot::Undefined) | None => Err(VmError::TclError {
                message: format!("can't read \"{}\": no such variable", name.as_str_lossy()),
                error_info: None,
                error_code: None,
            }),
            Some(VarSlot::Array(_)) => Err(VmError::TclError {
                message: format!("can't read \"{}\": variable is array", name.as_str_lossy()),
                error_info: None,
                error_code: None,
            }),
            Some(VarSlot::Link(_)) => Err(VmError::Internal(
                "load_scalar_local called on Link (resolve first)".into(),
            )),
        }
    }

    pub fn install_link(&mut self, name: ByteString, link: VarLink) {
        self.slots.insert(name, VarSlot::Link(link));
    }

    pub fn iter_slots(&self) -> impl Iterator<Item = (&ByteString, &VarSlot)> {
        self.slots.iter()
    }
}

/// Compiled local slot in a call frame.
#[derive(Debug, Clone)]
pub struct LocalSlot {
    pub name: ByteString,
    pub flags: i32,
    pub value: Option<VmObj>,
    pub link: Option<VarLink>,
}

impl LocalSlot {
    pub fn undefined(name: ByteString, flags: i32) -> Self {
        Self {
            name,
            flags,
            value: None,
            link: None,
        }
    }
}

/// Resolve a variable name that may use `arr(elem)` syntax into primary + optional part2.
pub fn split_var_name(name: &ByteString) -> (ByteString, Option<ByteString>) {
    parse_array_name(name.as_bytes())
}
