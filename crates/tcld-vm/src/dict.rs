//! Tcl dictionaries on VM values (`tclDictObj.c`): an insertion-ordered map
//! from key strings to values. Any list-shaped value reads as a dictionary,
//! and every update yields the canonical key/value list.

use std::collections::HashMap;

use crate::error::VmError;
use crate::list_codec::{ListKind, TclListCodec, find_element};
use crate::number::{self, Number};
use crate::value::VmObj;

#[derive(Debug, Clone, Default)]
pub struct Dict {
    entries: Vec<(VmObj, VmObj)>,
    positions: HashMap<Vec<u8>, usize>,
}

fn key_bytes(key: &VmObj) -> Result<Vec<u8>, VmError> {
    Ok(key.ensure_bytes()?.as_bytes().to_vec())
}

fn missing_value() -> VmError {
    VmError::TclError {
        message: "missing value to go with key".into(),
        error_info: None,
        error_code: Some("TCL VALUE DICTIONARY".into()),
    }
}

/// `key "k" not known in dictionary`, as a failed path lookup reports it.
pub fn unknown_key(key: &VmObj) -> Result<VmError, VmError> {
    let key = key.ensure_bytes()?;
    Ok(VmError::TclError {
        message: format!("key \"{}\" not known in dictionary", key.as_str_lossy()),
        error_info: None,
        error_code: Some(
            String::from_utf8_lossy(&TclListCodec::format([
                b"TCL".as_slice(),
                b"LOOKUP",
                b"DICT",
                key.as_bytes(),
            ]))
            .into_owned(),
        ),
    })
}

impl Dict {
    /// `SetDictFromAny`: a list value pairs up its elements and a string
    /// parses with dictionary syntax. A repeated key keeps its first
    /// position and its last value.
    pub fn from_value(value: &VmObj) -> Result<Self, VmError> {
        let mut dict = Self::default();
        if let VmObj::List(items) = value {
            if items.len() % 2 != 0 {
                return Err(missing_value());
            }
            for pair in items.chunks(2) {
                dict.put(pair[0].clone(), pair[1].clone())?;
            }
            return Ok(dict);
        }
        let text = value.ensure_bytes()?;
        let bytes = text.as_bytes();
        let element =
            |start| find_element(bytes, start).map_err(|error| error.into_vm_error(ListKind::Dict));
        let mut cursor = 0;
        while let Some((key, next)) = element(cursor)? {
            if next == bytes.len() {
                return Err(missing_value());
            }
            let Some((value, after)) = element(next)? else {
                return Err(missing_value());
            };
            dict.put(VmObj::from_bytes(key), VmObj::from_bytes(value))?;
            cursor = after;
        }
        Ok(dict)
    }

    /// The dictionary in a variable, or an empty one when it is unset.
    pub fn from_optional(value: Option<&VmObj>) -> Result<Self, VmError> {
        value.map_or_else(|| Ok(Self::default()), Self::from_value)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[(VmObj, VmObj)] {
        &self.entries
    }

    pub fn into_entries(self) -> Vec<(VmObj, VmObj)> {
        self.entries
    }

    pub fn keys(&self) -> Vec<VmObj> {
        self.entries.iter().map(|(key, _)| key.clone()).collect()
    }

    pub fn get(&self, key: &VmObj) -> Result<Option<&VmObj>, VmError> {
        Ok(self
            .positions
            .get(&key_bytes(key)?)
            .map(|&position| &self.entries[position].1))
    }

    /// Replace the value of an existing key in place, or append the key.
    pub fn put(&mut self, key: VmObj, value: VmObj) -> Result<(), VmError> {
        let bytes = key_bytes(&key)?;
        match self.positions.get(&bytes) {
            Some(&position) => self.entries[position].1 = value,
            None => {
                self.positions.insert(bytes, self.entries.len());
                self.entries.push((key, value));
            }
        }
        Ok(())
    }

    pub fn remove(&mut self, key: &VmObj) -> Result<(), VmError> {
        if let Some(position) = self.positions.remove(&key_bytes(key)?) {
            self.entries.remove(position);
            for later in self.positions.values_mut() {
                if *later > position {
                    *later -= 1;
                }
            }
        }
        Ok(())
    }

    /// The canonical value: the key/value list.
    pub fn into_value(self) -> VmObj {
        VmObj::List(
            self.entries
                .into_iter()
                .flat_map(|(key, value)| [key, value])
                .collect(),
        )
    }

    /// `TclTraceDictPath` with `DICT_PATH_READ`: the dictionary reached
    /// through `path`, where every key must exist.
    pub fn trace(value: &VmObj, path: &[VmObj]) -> Result<Self, VmError> {
        let mut dict = Self::from_value(value)?;
        for key in path {
            dict = match dict.get(key)? {
                Some(inner) => Self::from_value(inner)?,
                None => return Err(unknown_key(key)?),
            };
        }
        Ok(dict)
    }

    /// `TclTraceDictPath` with `DICT_PATH_EXISTS`: `None` when a key on the
    /// path is missing.
    pub fn trace_existing(value: &VmObj, path: &[VmObj]) -> Result<Option<Self>, VmError> {
        let mut dict = Self::from_value(value)?;
        for key in path {
            dict = match dict.get(key)? {
                Some(inner) => Self::from_value(inner)?,
                None => return Ok(None),
            };
        }
        Ok(Some(dict))
    }

    /// `Tcl_DictObjPutKeyList`: set the value at a key path, creating
    /// missing intermediate dictionaries.
    pub fn put_path(&mut self, keys: &[VmObj], value: VmObj) -> Result<(), VmError> {
        let Some((key, rest)) = keys.split_first() else {
            return Err(VmError::Internal("empty dictionary key path".into()));
        };
        if rest.is_empty() {
            return self.put(key.clone(), value);
        }
        let mut inner = Self::from_optional(self.get(key)?)?;
        inner.put_path(rest, value)?;
        self.put(key.clone(), inner.into_value())
    }

    /// `Tcl_DictObjRemoveKeyList`: remove the last key of a path whose
    /// intermediate keys must exist.
    pub fn remove_path(&mut self, keys: &[VmObj]) -> Result<(), VmError> {
        let Some((key, rest)) = keys.split_first() else {
            return Err(VmError::Internal("empty dictionary key path".into()));
        };
        if rest.is_empty() {
            return self.remove(key);
        }
        let mut inner = match self.get(key)? {
            Some(inner) => Self::from_value(inner)?,
            None => return Err(unknown_key(key)?),
        };
        inner.remove_path(rest)?;
        self.put(key.clone(), inner.into_value())
    }

    /// `dict incr` (`INST_DICT_INCR_IMM`): a missing key starts at the
    /// increment; an existing value must be an integer.
    pub fn incr(&mut self, key: &VmObj, amount: i64) -> Result<(), VmError> {
        let value = match self.get(key)? {
            None => VmObj::Int(amount),
            Some(current) => match number::of(current) {
                Some(Number::Int(value)) => value.checked_add(amount).map_or_else(
                    || VmObj::from_bigint(num_bigint::BigInt::from(value) + amount),
                    VmObj::Int,
                ),
                Some(Number::Big(value)) => VmObj::from_bigint(value + amount),
                _ => return Err(number::expected("integer", current)),
            },
        };
        self.put(key.clone(), value)
    }

    /// `dict append`: concatenate onto the existing value.
    pub fn append(&mut self, key: &VmObj, tail: &VmObj) -> Result<(), VmError> {
        let value = match self.get(key)? {
            None => tail.clone(),
            Some(current) => {
                let mut bytes = current.ensure_bytes()?.as_bytes().to_vec();
                bytes.extend_from_slice(tail.ensure_bytes()?.as_bytes());
                VmObj::from_bytes(bytes)
            }
        };
        self.put(key.clone(), value)
    }

    /// `dict lappend`: append one element to the existing list value.
    pub fn lappend(&mut self, key: &VmObj, element: &VmObj) -> Result<(), VmError> {
        let mut items = match self.get(key)? {
            None => Vec::new(),
            Some(current) => current.as_list_vec()?,
        };
        items.push(element.clone());
        self.put(key.clone(), VmObj::List(items))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: VmObj) -> String {
        value.as_string().unwrap()
    }

    fn error(result: Result<Dict, VmError>) -> (String, Option<String>) {
        match result.unwrap_err() {
            VmError::TclError {
                message,
                error_code,
                ..
            } => (message, error_code),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn repeated_keys_keep_first_position_and_last_value() {
        let dict = Dict::from_value(&VmObj::from_str("a 1 b 2 a 3")).unwrap();
        assert_eq!(dict.len(), 2);
        assert_eq!(text(dict.into_value()), "a 3 b 2");
    }

    #[test]
    fn malformed_dictionaries_report_like_tcl() {
        assert_eq!(
            error(Dict::from_value(&VmObj::from_str("a b c"))),
            (
                "missing value to go with key".into(),
                Some("TCL VALUE DICTIONARY".into())
            )
        );
        assert_eq!(
            error(Dict::from_value(&VmObj::from_str("a {b"))),
            (
                "unmatched open brace in dict".into(),
                Some("TCL VALUE DICTIONARY BRACE".into())
            )
        );
        assert_eq!(
            error(Dict::from_value(&VmObj::List(vec![VmObj::from_str("k")]))).0,
            "missing value to go with key"
        );
    }

    #[test]
    fn paths_create_update_and_remove() {
        let mut dict = Dict::from_value(&VmObj::from_str("a {x 1}")).unwrap();
        dict.put_path(
            &[VmObj::from_str("a"), VmObj::from_str("y")],
            VmObj::from_str("two words"),
        )
        .unwrap();
        dict.put_path(&[VmObj::from_str("b"), VmObj::from_str("c")], VmObj::Int(3))
            .unwrap();
        assert_eq!(
            text(dict.clone().into_value()),
            "a {x 1 y {two words}} b {c 3}"
        );
        dict.remove_path(&[VmObj::from_str("a"), VmObj::from_str("x")])
            .unwrap();
        assert_eq!(text(dict.clone().into_value()), "a {y {two words}} b {c 3}");
        let missing = dict
            .remove_path(&[VmObj::from_str("q"), VmObj::from_str("x")])
            .unwrap_err();
        assert!(matches!(
            missing,
            VmError::TclError { message, error_code: Some(code), .. }
                if message == "key \"q\" not known in dictionary" && code == "TCL LOOKUP DICT q"
        ));
    }

    #[test]
    fn incr_append_and_lappend() {
        let mut dict = Dict::default();
        let key = VmObj::from_str("n");
        dict.incr(&key, 5).unwrap();
        dict.incr(&key, i64::MAX).unwrap();
        dict.append(&VmObj::from_str("s"), &VmObj::from_str("ab"))
            .unwrap();
        dict.append(&VmObj::from_str("s"), &VmObj::from_str("c"))
            .unwrap();
        dict.lappend(&VmObj::from_str("l"), &VmObj::from_str("x y"))
            .unwrap();
        assert_eq!(
            text(dict.clone().into_value()),
            "n 9223372036854775812 s abc l {{x y}}"
        );
        dict.put(VmObj::from_str("f"), VmObj::from_str("1.5"))
            .unwrap();
        assert!(dict.incr(&VmObj::from_str("f"), 1).is_err());
    }
}
