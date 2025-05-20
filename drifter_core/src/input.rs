/// Represents a single input for the fuzzer.
///
/// Inputs are the basic units of data that are fed to the target program being fuzzed.
/// Implementors of this trait define how their specific input type can be
/// represented as raw bytes, its length, and whether it's empty.
///
/// The `'static` lifetime bound, along with `Send` and `Sync`, allows `dyn Input`
/// trait objects to be versatile, for example, by being stored in collections shared
/// across threads or passed to different components of the fuzzer.
pub trait Input: Send + Sync + std::fmt::Debug + 'static {
    /// Returns the input data as a byte slice.
    ///
    /// This is the primary method used by `Executor`s to obtain the raw data
    /// to pass to the target program.
    fn as_bytes(&self) -> &[u8];

    /// Returns the length of the input in bytes.
    ///
    /// This corresponds to the length of the slice returned by `as_bytes()`.
    fn len(&self) -> usize;

    /// Returns `true` if the input has a length of zero bytes.
    ///
    /// This is equivalent to `self.len() == 0`.
    fn is_empty(&self) -> bool;
}

/// Default implementation of the `Input` trait for `Vec<u8>`.
///
/// This allows a simple vector of bytes to be directly used as a fuzzing input type,
/// which is common for many fuzzing scenarios.
impl Input for Vec<u8> {
    /// Returns the `Vec<u8>` as a byte slice.
    #[inline]
    fn as_bytes(&self) -> &[u8] {
        self.as_slice()
    }

    /// Returns the length of the `Vec<u8>`.
    #[inline]
    fn len(&self) -> usize {
        Vec::len(self)
    }

    /// Checks if the `Vec<u8>` is empty.
    #[inline]
    fn is_empty(&self) -> bool {
        Vec::is_empty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_u8_implements_input_trait_correctly() {
        let data: Vec<u8> = vec![1, 2, 3];
        let empty_data: Vec<u8> = vec![];

        assert_eq!(data.as_bytes(), &[1, 2, 3], "as_bytes() for non-empty vec");
        assert_eq!(
            empty_data.as_bytes(),
            &[] as &[u8],
            "as_bytes() for empty vec"
        );

        assert_eq!(data.len(), 3, "len() for non-empty vec");
        assert_eq!(empty_data.len(), 0, "len() for empty vec");

        assert!(
            !data.is_empty(),
            "is_empty() should be false for non-empty vec"
        );
        assert!(
            empty_data.is_empty(),
            "is_empty() should be true for empty vec"
        );
    }
}
