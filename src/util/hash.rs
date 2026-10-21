//! The hash of FNV-1a.
//!
//! The port derives the UUID of the world, of the player and of the
//! entities from the seed of the world and from the name of the player.
//! The reference draws them at random, but the files of the world have to
//! stay the same from run to run.

/// The hash of FNV-1a (sixty four bits).
pub fn fnv1a(bytes: &[u8], seed: u64) -> i64 {
    let mut hash = seed;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash as i64
}

/// The UUID of the world (`World.getUUID()`).
///
/// The reference draws the UUID of the world at random, the port derives
/// it from the seed of the world, so that the files of the world are the
/// same from run to run.
pub fn world_uuid(seed: i64) -> (i64, i64) {
    (
        fnv1a(&seed.to_le_bytes(), 0xcbf2_9ce4_8422_2325),
        fnv1a(&seed.to_le_bytes(), 0x9e37_79b9_7f4a_7c15),
    )
}

/// The UUID of the player (`Entity.uniqueId`).
///
/// The reference draws it at random, the port derives it from the name of
/// the player.
pub fn player_uuid(name: &str) -> (i64, i64) {
    (
        fnv1a(name.as_bytes(), 0xcbf2_9ce4_8422_2325),
        fnv1a(name.as_bytes(), 0x9e37_79b9_7f4a_7c15),
    )
}

/// The UUID of the entity (`Entity.uniqueId`).
///
/// The reference draws it at random, the port derives it from the seed of
/// the world and from the identifier of the entity.
pub fn entity_uuid(seed: i64, entity_id: i32) -> (i64, i64) {
    let bytes = (seed as u64)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(entity_id as u64)
        .to_le_bytes();
    (
        fnv1a(&bytes, 0xcbf2_9ce4_8422_2325),
        fnv1a(&bytes, 0x9e37_79b9_7f4a_7c15),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_of_the_uuid_is_stable() {
        assert_eq!(world_uuid(12345), world_uuid(12345));
        assert_ne!(world_uuid(12345), world_uuid(12346));
        assert_eq!(player_uuid("Steve"), player_uuid("Steve"));
        assert_ne!(player_uuid("Steve"), player_uuid("Alex"));
        assert_eq!(entity_uuid(1, 7), entity_uuid(1, 7));
        assert_ne!(entity_uuid(1, 7), entity_uuid(1, 8));
    }
}
