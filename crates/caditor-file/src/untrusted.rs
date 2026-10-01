#[expect(
    clippy::disallowed_types,
    reason = "keys read from a file keep the standard DoS-resistant hasher"
)]
pub(crate) type UntrustedMap<K, V> = std::collections::HashMap<K, V>;

#[expect(
    clippy::disallowed_types,
    reason = "keys read from a file keep the standard DoS-resistant hasher"
)]
pub(crate) type UntrustedSet<T> = std::collections::HashSet<T>;
