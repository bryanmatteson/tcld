use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt::Write;
use std::rc::Rc;

pub trait SourceRepresentable {
    fn write_source_repr(&self, out: &mut String);

    fn source_repr(&self) -> String {
        let mut out = String::new();
        self.write_source_repr(&mut out);
        out
    }
}

#[derive(Clone)]
pub enum SourceRef {
    Static(&'static str),
    Shared(Rc<str>),
    Int(i32),
    Int64(i64),
    Float32(f32),
    Float64(f64),
    Bool(bool),
    Dyn(Rc<dyn SourceRepresentable>),
}

impl SourceRef {
    pub fn source_repr(&self) -> String {
        let mut out = String::new();
        self.write_repr(&mut out);
        out
    }

    pub fn write_repr(&self, out: &mut String) {
        match self {
            SourceRef::Static(s) => out.push_str(s),
            SourceRef::Shared(s) => out.push_str(s),
            SourceRef::Int(v) => {
                let _ = write!(out, "{}", v);
            }
            SourceRef::Int64(v) => {
                let _ = write!(out, "{}", v);
            }
            SourceRef::Float32(v) => {
                let _ = write!(out, "{}", v);
            }
            SourceRef::Float64(v) => {
                let _ = write!(out, "{}", v);
            }
            SourceRef::Bool(v) => {
                let _ = write!(out, "{}", v);
            }
            SourceRef::Dyn(v) => v.write_source_repr(out),
        }
    }

    pub fn repr_cow(&self) -> Cow<'_, str> {
        match self {
            SourceRef::Static(s) => Cow::Borrowed(s),
            SourceRef::Shared(s) => Cow::Borrowed(s),
            SourceRef::Int(v) => Cow::Owned(v.to_string()),
            SourceRef::Int64(v) => Cow::Owned(v.to_string()),
            SourceRef::Float32(v) => Cow::Owned(v.to_string()),
            SourceRef::Float64(v) => Cow::Owned(v.to_string()),
            SourceRef::Bool(v) => Cow::Owned(v.to_string()),
            SourceRef::Dyn(v) => {
                let mut out = String::new();
                v.write_source_repr(&mut out);
                Cow::Owned(out)
            }
        }
    }

    pub fn repr_eq(&self, other: &SourceRef) -> bool {
        match (self, other) {
            (SourceRef::Static(a), SourceRef::Static(b)) => a == b,
            (SourceRef::Shared(a), SourceRef::Shared(b)) => a == b,
            (SourceRef::Static(a), SourceRef::Shared(b)) => *a == b.as_ref(),
            (SourceRef::Shared(a), SourceRef::Static(b)) => a.as_ref() == *b,
            (SourceRef::Int(a), SourceRef::Int(b)) => a == b,
            (SourceRef::Int64(a), SourceRef::Int64(b)) => a == b,
            (SourceRef::Int(a), SourceRef::Int64(b)) => (*a as i64) == *b,
            (SourceRef::Int64(a), SourceRef::Int(b)) => *a == (*b as i64),
            (SourceRef::Float32(a), SourceRef::Float32(b)) => a == b,
            (SourceRef::Float64(a), SourceRef::Float64(b)) => a == b,
            (SourceRef::Bool(a), SourceRef::Bool(b)) => a == b,
            _ => self.repr_cow() == other.repr_cow(),
        }
    }

    pub fn repr_eq_str(&self, rhs: &str) -> bool {
        match self {
            SourceRef::Static(s) => *s == rhs,
            SourceRef::Shared(s) => s.as_ref() == rhs,
            SourceRef::Int(v) => rhs.parse::<i32>().is_ok_and(|x| x == *v),
            SourceRef::Int64(v) => rhs.parse::<i64>().is_ok_and(|x| x == *v),
            SourceRef::Float32(v) => rhs.parse::<f32>().is_ok_and(|x| x == *v),
            SourceRef::Float64(v) => rhs.parse::<f64>().is_ok_and(|x| x == *v),
            SourceRef::Bool(v) => rhs.parse::<bool>().is_ok_and(|x| x == *v),
            SourceRef::Dyn(v) => {
                let mut out = String::with_capacity(rhs.len());
                v.write_source_repr(&mut out);
                out == rhs
            }
        }
    }

    pub fn repr_is_empty(&self) -> bool {
        match self {
            SourceRef::Static(s) => s.is_empty(),
            SourceRef::Shared(s) => s.is_empty(),
            _ => self.repr_eq_str(""),
        }
    }

    pub fn repr_parse_i64(&self) -> Option<i64> {
        match self {
            SourceRef::Int(v) => Some(*v as i64),
            SourceRef::Int64(v) => Some(*v),
            SourceRef::Static(s) => s.parse::<i64>().ok(),
            SourceRef::Shared(s) => s.parse::<i64>().ok(),
            SourceRef::Dyn(v) => {
                let mut out = String::new();
                v.write_source_repr(&mut out);
                out.parse::<i64>().ok()
            }
            _ => None,
        }
    }

    pub fn repr_cmp(&self, other: &SourceRef) -> Ordering {
        match (self, other) {
            (SourceRef::Static(a), SourceRef::Static(b)) => a.cmp(b),
            (SourceRef::Shared(a), SourceRef::Shared(b)) => a.as_ref().cmp(b.as_ref()),
            (SourceRef::Static(a), SourceRef::Shared(b)) => a.cmp(&b.as_ref()),
            (SourceRef::Shared(a), SourceRef::Static(b)) => a.as_ref().cmp(b),
            (SourceRef::Int(a), SourceRef::Int(b)) => a.cmp(b),
            (SourceRef::Int64(a), SourceRef::Int64(b)) => a.cmp(b),
            (SourceRef::Int(a), SourceRef::Int64(b)) => (*a as i64).cmp(b),
            (SourceRef::Int64(a), SourceRef::Int(b)) => a.cmp(&(*b as i64)),
            _ => self.repr_cow().cmp(&other.repr_cow()),
        }
    }
}

impl SourceRepresentable for SourceRef {
    fn source_repr(&self) -> String {
        SourceRef::source_repr(self)
    }

    fn write_source_repr(&self, out: &mut String) {
        self.write_repr(out);
    }
}

pub trait IntoSourceRef {
    fn into_source_ref(self) -> SourceRef;
}

impl IntoSourceRef for SourceRef {
    fn into_source_ref(self) -> SourceRef {
        self
    }
}

impl IntoSourceRef for String {
    fn into_source_ref(self) -> SourceRef {
        SourceRef::Shared(Rc::from(self.into_boxed_str()))
    }
}

impl IntoSourceRef for &'static str {
    fn into_source_ref(self) -> SourceRef {
        SourceRef::Static(self)
    }
}

impl IntoSourceRef for i32 {
    fn into_source_ref(self) -> SourceRef {
        SourceRef::Int(self)
    }
}

impl IntoSourceRef for i64 {
    fn into_source_ref(self) -> SourceRef {
        SourceRef::Int64(self)
    }
}

impl IntoSourceRef for f32 {
    fn into_source_ref(self) -> SourceRef {
        SourceRef::Float32(self)
    }
}

impl IntoSourceRef for f64 {
    fn into_source_ref(self) -> SourceRef {
        SourceRef::Float64(self)
    }
}

impl IntoSourceRef for bool {
    fn into_source_ref(self) -> SourceRef {
        SourceRef::Bool(self)
    }
}

pub fn sr<T: IntoSourceRef>(value: T) -> SourceRef {
    value.into_source_ref()
}

pub fn sr_dyn<T: SourceRepresentable + 'static>(value: T) -> SourceRef {
    SourceRef::Dyn(Rc::new(value))
}
