use std::borrow::Cow;

/// Fully-qualified component name, e.g. "orts.Position3D".
pub type ComponentName = Cow<'static, str>;

/// A component is the smallest unit of data that can be logged for an entity.
///
/// Components are value types that know their name and can serialize themselves
/// to a flat f64 slice (for Arrow Float64Array compatibility).
pub trait Component: Clone + std::fmt::Debug + Send + Sync + 'static {
    /// Fully-qualified component name.
    fn component_name() -> ComponentName;

    /// Number of f64 values per instance of this component.
    fn num_scalars() -> usize;

    /// Serialize this component instance to a flat f64 slice.
    fn to_scalars(&self) -> Vec<f64>;

    /// Deserialize from a flat f64 slice. Returns None if the slice is wrong length.
    fn from_scalars(data: &[f64]) -> Option<Self>;

    /// Column names for Arrow schema generation.
    fn field_names() -> Vec<&'static str>;
}
