/// Hierarchical entity identifier using `/`-separated path segments.
///
/// Examples: "/world/earth", "/world/sat/iss", "/world/station/tanegashima"
///
/// Serializes as a `/`-prefixed string (e.g., `"/world/sat/iss"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntityPath {
    segments: Vec<String>,
}

impl serde::Serialize for EntityPath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for EntityPath {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(EntityPath::parse(&s))
    }
}

impl EntityPath {
    /// Parse from a `/`-separated string. Leading `/` is optional.
    pub fn parse(path: &str) -> Self {
        let segments: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        EntityPath { segments }
    }

    /// Return the parent entity path, or None if this is root-level.
    pub fn parent(&self) -> Option<EntityPath> {
        if self.segments.len() <= 1 {
            return None;
        }
        Some(EntityPath {
            segments: self.segments[..self.segments.len() - 1].to_vec(),
        })
    }

    /// Return the leaf name.
    pub fn name(&self) -> &str {
        self.segments.last().map(|s| s.as_str()).unwrap_or("")
    }

    /// Append a child segment.
    pub fn join(&self, child: &str) -> EntityPath {
        let mut segments = self.segments.clone();
        segments.push(child.to_string());
        EntityPath { segments }
    }
}

impl std::fmt::Display for EntityPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "/{}", self.segments.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn serialize_json() {
        let path = EntityPath::parse("/world/sat/iss");
        let json = serde_json::to_string(&path).unwrap();
        assert_eq!(json, r#""/world/sat/iss""#);
    }

    #[test]
    fn deserialize_json() {
        let path: EntityPath = serde_json::from_str(r#""/world/sat/iss""#).unwrap();
        assert_eq!(path, EntityPath::parse("/world/sat/iss"));
    }

    #[test]
    fn serde_roundtrip() {
        let original = EntityPath::parse("/world/sat/apollo11");
        let json = serde_json::to_string(&original).unwrap();
        let restored: EntityPath = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn parse_absolute_path() {
        let path = EntityPath::parse("/world/earth");
        assert_eq!(path.to_string(), "/world/earth");
        assert_eq!(path.name(), "earth");
    }

    #[test]
    fn parse_without_leading_slash() {
        let path = EntityPath::parse("world/earth");
        assert_eq!(path.to_string(), "/world/earth");
    }

    #[test]
    fn parse_single_segment() {
        let path = EntityPath::parse("/world");
        assert_eq!(path.name(), "world");
        assert_eq!(path.parent(), None);
    }

    #[test]
    fn parent() {
        let path = EntityPath::parse("/world/sat/iss");
        let parent = path.parent().unwrap();
        assert_eq!(parent.to_string(), "/world/sat");

        let grandparent = parent.parent().unwrap();
        assert_eq!(grandparent.to_string(), "/world");

        assert_eq!(grandparent.parent(), None);
    }

    #[test]
    fn join() {
        let world = EntityPath::parse("/world");
        let sat = world.join("sat");
        assert_eq!(sat.to_string(), "/world/sat");

        let iss = sat.join("iss");
        assert_eq!(iss.to_string(), "/world/sat/iss");
    }

    #[test]
    fn equality() {
        let a = EntityPath::parse("/world/earth");
        let b = EntityPath::parse("world/earth");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_equality() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(EntityPath::parse("/world/earth"));
        assert!(set.contains(&EntityPath::parse("world/earth")));
    }

    #[test]
    fn display() {
        let path = EntityPath::parse("/solar_system/earth/sat/iss");
        assert_eq!(format!("{path}"), "/solar_system/earth/sat/iss");
    }
}
