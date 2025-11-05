/// Entity is just a unique identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity(pub u64);

impl Entity {
    pub fn new(id: u64) -> Self {
        Entity(id)
    }

    pub fn id(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_entity_creation() {
        let entity = Entity::new(42);
        assert_eq!(entity.id(), 42);
    }

    #[test]
    fn test_entity_equality() {
        let e1 = Entity::new(1);
        let e2 = Entity::new(1);
        let e3 = Entity::new(2);

        assert_eq!(e1, e2);
        assert_ne!(e1, e3);
    }

    #[test]
    fn test_entity_copy() {
        let e1 = Entity::new(5);
        let e2 = e1; // Should copy, not move
        assert_eq!(e1, e2);
    }
}
