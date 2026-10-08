use super::ReadingError;

// ByteBufCodecs.collection/map cap initial allocation, not the decoded list length.
pub const MAX_INITIAL_COLLECTION_SIZE: usize = 65_536;

/// Checks signed collection counts and caps only the initial allocation.
pub fn collection_capacity(count: impl TryInto<usize>) -> Result<usize, ReadingError> {
    let count = count
        .try_into()
        .map_err(|_| ReadingError::Message("Negative collection length".into()))?;
    if count > i32::MAX as usize {
        return Err(ReadingError::TooLarge("Collection length".into()));
    }
    super::decode_budget::charge_collection_work(count)?;
    Ok(count.min(MAX_INITIAL_COLLECTION_SIZE))
}

#[cfg(test)]
mod tests {
    use crate::ser::NetworkReadExt;

    #[test]
    fn oversized_collection_is_read_incrementally() {
        let _scope = crate::ser::decode_budget::DecodeScope::packet();
        // VarInt 65,537 and one element: the second element must report EOF.
        let mut input: &[u8] = &[0x81, 0x80, 0x04, 0x01];
        assert!(matches!(
            input.get_list(NetworkReadExt::get_u8),
            Err(super::ReadingError::Incomplete(_))
        ));
    }
}
