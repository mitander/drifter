pub trait Input: Clone + Send + Sync + std::fmt::Debug + 'static {
    fn as_bytes(&self) -> &[u8];
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}

impl Input for Vec<u8> {
    fn as_bytes(&self) -> &[u8] {
        self.as_slice()
    }
    fn len(&self) -> usize {
        self.len()
    }
    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vec_u8_impl_input() {
        let data: Vec<u8> = vec![1, 2, 3];
        let empty_data: Vec<u8> = vec![];
        assert_eq!(data.as_bytes(), &[1, 2, 3]);
        assert_eq!(data.len(), 3);
        assert!(!data.is_empty());
        assert!(empty_data.is_empty());
    }
}
