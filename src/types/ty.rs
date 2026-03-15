#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    Number,
    String,
    Boolean,
    Void,
    Null,
    Undefined,
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
    },
    Array(Box<Type>),
    /// Object with known field names and types (ordered)
    Object {
        fields: Vec<(String, Type)>,
    },
    /// A named class type. `fields` stores the instance fields + methods.
    Class {
        name: String,
        fields: Vec<(String, Type)>,
    },
    // Used internally when a type cannot be determined
    Unknown,
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Number => write!(f, "number"),
            Type::String => write!(f, "string"),
            Type::Boolean => write!(f, "boolean"),
            Type::Void => write!(f, "void"),
            Type::Null => write!(f, "null"),
            Type::Undefined => write!(f, "undefined"),
            Type::Function {
                params,
                return_type,
            } => {
                write!(f, "(")?;
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", p)?;
                }
                write!(f, ") => {}", return_type)
            }
            Type::Array(elem) => write!(f, "{}[]", elem),
            Type::Object { fields } => {
                write!(f, "{{ ")?;
                for (i, (name, ty)) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", name, ty)?;
                }
                write!(f, " }}")
            }
            Type::Class { name, .. } => write!(f, "{}", name),
            Type::Unknown => write!(f, "unknown"),
        }
    }
}
