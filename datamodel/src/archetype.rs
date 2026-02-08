use crate::component::ComponentName;

/// An archetype is a named bundle of components that are commonly logged together.
pub trait Archetype: std::fmt::Debug + Send + Sync {
    /// Human-readable archetype name, e.g. "OrbitalState".
    fn archetype_name() -> &'static str;

    /// The component names that are required for this archetype.
    fn required_components() -> Vec<ComponentName>;

    /// The component names that are optional (but semantically related).
    fn optional_components() -> Vec<ComponentName>;
}
