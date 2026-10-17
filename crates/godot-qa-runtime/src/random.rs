//! Small randomness source for session tokens, session ids, and ephemeral-port
//! fallbacks. Uses the OS CSPRNG via the [`getrandom`] crate (works uniformly on
//! Unix, Windows, and other supported targets — unlike a raw `/dev/urandom`
//! read, which does not exist on Windows) and only falls back to a best-effort
//! entropy mix when that call fails, which effectively never happens on a
//! healthy host. This gives `docs/security-model.md`'s "random per-session
//! token" promise a real randomness source instead of a time-derived,
//! guessable value.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Fill `buf` with random bytes from the OS CSPRNG, falling back to a
/// best-effort entropy mix only if the underlying `getrandom` call fails.
pub fn random_bytes(buf: &mut [u8]) {
    if getrandom::fill(buf).is_ok() {
        return;
    }
    fill_fallback(buf);
}

/// Last-resort entropy when the OS CSPRNG is unreachable: mix wall-clock nanos,
/// the pid, and a stack address through a SplitMix64 stream. Not cryptographic,
/// but avoids the predictable constant output a time-only source would produce.
fn fill_fallback(buf: &mut [u8]) {
    // Monotonic per-call counter so two calls that land in the same clock tick
    // (coarse timers) still produce distinct output.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let pid = u64::from(std::process::id());
    let stack_marker = &nanos as *const u64 as u64;
    let counter = COUNTER
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let mut state = nanos ^ pid.rotate_left(32) ^ stack_marker ^ counter;
    for chunk in buf.chunks_mut(8) {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        for (dst, src) in chunk.iter_mut().zip(z.to_le_bytes()) {
            *dst = src;
        }
    }
}

/// Return `2 * n_bytes` lowercase hex characters sourced from [`random_bytes`].
pub fn random_hex(n_bytes: usize) -> String {
    let mut bytes = vec![0u8; n_bytes];
    random_bytes(&mut bytes);
    let mut out = String::with_capacity(n_bytes * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        _ => (b'a' + (nibble - 10)) as char,
    }
}

/// Random `u16` sourced from [`random_bytes`]. Used for the ephemeral-port
/// fallback when the kernel refuses an ephemeral bind.
pub fn random_u16() -> u16 {
    let mut bytes = [0u8; 2];
    random_bytes(&mut bytes);
    u16::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn random_hex_has_expected_length_and_charset() {
        let value = random_hex(16);
        assert_eq!(value.len(), 32);
        assert!(
            value
                .chars()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
        );
    }

    #[test]
    fn random_hex_differs_across_generations() {
        let mut seen = HashSet::new();
        for _ in 0..64 {
            assert!(seen.insert(random_hex(16)), "random_hex(16) collided");
        }
    }

    #[test]
    fn fill_fallback_is_not_constant() {
        let mut a = [0u8; 16];
        let mut b = [0u8; 16];
        fill_fallback(&mut a);
        fill_fallback(&mut b);
        assert_ne!(a, b, "fallback entropy must not be constant");
    }
}
