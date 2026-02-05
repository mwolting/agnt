use crate::request::Schema;

/// Trait for types that can describe their shape as a [`Schema`].
///
/// This is the Rust analog of zod's `.describe()` — it produces a schema
/// descriptor that language models use to understand the expected JSON input.
///
/// Implement this manually for now; a derive macro will be added later.
///
/// # Example
///
/// ```
/// use agnt_llm::{Describe, Schema, Property};
///
/// struct ReadFileInput {
///     path: String,
///     offset: Option<usize>,
/// }
///
/// impl Describe for ReadFileInput {
///     fn describe() -> Schema {
///         Schema::Object {
///             description: Some("Read a file from disk".into()),
///             properties: vec![
///                 Property {
///                     name: "path".into(),
///                     schema: Schema::String { description: Some("File path".into()), enumeration: None },
///                 },
///                 Property {
///                     name: "offset".into(),
///                     schema: Schema::Integer { description: Some("Line offset".into()) },
///                 },
///             ],
///             required: vec!["path".into()],
///         }
///     }
/// }
/// ```
pub trait Describe {
    /// Return a [`Schema`] describing this type's structure.
    fn describe() -> Schema;
}

// ---------------------------------------------------------------------------
// Built-in impls for common types
// ---------------------------------------------------------------------------

impl Describe for String {
    fn describe() -> Schema {
        Schema::String {
            description: None,
            enumeration: None,
        }
    }
}

impl Describe for bool {
    fn describe() -> Schema {
        Schema::Boolean { description: None }
    }
}

impl Describe for f64 {
    fn describe() -> Schema {
        Schema::Number { description: None }
    }
}

impl Describe for f32 {
    fn describe() -> Schema {
        Schema::Number { description: None }
    }
}

impl Describe for i64 {
    fn describe() -> Schema {
        Schema::Integer { description: None }
    }
}

impl Describe for i32 {
    fn describe() -> Schema {
        Schema::Integer { description: None }
    }
}

impl Describe for u64 {
    fn describe() -> Schema {
        Schema::Integer { description: None }
    }
}

impl Describe for u32 {
    fn describe() -> Schema {
        Schema::Integer { description: None }
    }
}

impl Describe for usize {
    fn describe() -> Schema {
        Schema::Integer { description: None }
    }
}

impl<T: Describe> Describe for Vec<T> {
    fn describe() -> Schema {
        Schema::Array {
            description: None,
            items: Box::new(T::describe()),
        }
    }
}
