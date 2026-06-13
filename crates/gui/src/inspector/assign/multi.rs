//! Folding a multi-selection into per-field common-or-indeterminate values.

/// One field's value across a selection: a shared value, or differing (`…`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MultiValue<T> {
    Same(T),
    Differ,
}

impl<T> MultiValue<T> {
    pub(crate) fn value(self) -> Option<T> {
        match self {
            MultiValue::Same(v) => Some(v),
            MultiValue::Differ => None,
        }
    }
}

/// Fold an iterator of field values into `Same(v)` when all equal (and at least
/// one), or `Differ` when any two differ or the iterator is empty.
pub(crate) fn fold<T: PartialEq>(iter: impl IntoIterator<Item = T>) -> MultiValue<T> {
    let mut it = iter.into_iter();
    let Some(first) = it.next() else {
        return MultiValue::Differ;
    };
    if it.all(|v| v == first) {
        MultiValue::Same(first)
    } else {
        MultiValue::Differ
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_values_fold_to_same() {
        assert_eq!(fold([3u8, 3, 3]), MultiValue::Same(3));
    }

    #[test]
    fn differing_values_fold_to_differ() {
        assert_eq!(fold([3u8, 4, 3]), MultiValue::Differ);
    }

    #[test]
    fn single_value_is_same() {
        assert_eq!(fold([7u8]), MultiValue::Same(7));
    }

    #[test]
    fn empty_is_differ() {
        assert_eq!(fold(std::iter::empty::<u8>()), MultiValue::Differ);
    }
}
