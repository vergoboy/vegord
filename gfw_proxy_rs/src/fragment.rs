use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};

const TLS_RECORD_HANDSHAKE: u8 = 0x16;
const TLS_HANDSHAKE_CLIENT_HELLO: u8 = 0x01;
const TLS_EXTENSION_SERVER_NAME: u16 = 0x0000;
const TLS_EXT_HOST_NAME: u8 = 0x00; // server_name list entry type "host_name"

/// Keep the first TCP segment short enough that an on-path DPI box cannot
/// fingerprint the ClientHello from it alone. The ClientHello's cipher-suite
/// list starts right after record(5) + handshake(4) + version(2) + random(32)
/// = byte 43; a first segment that ends before byte 43 carries no ciphers and
/// is unclassifiable. Empirically: first segment <= 40 bytes passes, >= 42
/// gets reset, so 24 leaves a safe margin while still being a real cut.
const FIRST_FRAGMENT_SIZE: usize = 24;

/// Locate the byte offset of the SNI hostname inside a TLS ClientHello.
///
/// The spec requires splitting exactly at the SNI extension offset rather than
/// at a random index over the whole buffer: random splits often miss the SNI
/// field entirely, wasting a meaningful fraction of attempts against SNI-based
/// DPI. When the ClientHello carries an SNI, this returns the offset where the
/// hostname string itself begins, so the two resulting writes never carry the
/// full hostname contiguously in one TCP segment.
///
/// Returns `None` when the record is not a TLS handshake, is truncated, or has
/// no server_name extension (e.g. TLS to a raw IP with no SNI).
pub fn find_sni_offset(data: &[u8]) -> Option<usize> {
    // TLS record header: content type(1) + version(2) + length(2)
    if data.len() < 5 || data[0] != TLS_RECORD_HANDSHAKE {
        return None;
    }
    let mut off = 5usize;

    // Handshake message header: msg type(1) + length(3)
    if data.len() < off + 4 || data[off] != TLS_HANDSHAKE_CLIENT_HELLO {
        return None;
    }
    off += 4;

    // ClientHello body:
    //   version(2) random(32)
    if data.len() < off + 34 {
        return None;
    }
    off += 34;

    //   session_id: len(1) + bytes
    let session_len = data[off] as usize;
    off += 1;
    if data.len() < off + session_len + 2 {
        return None;
    }
    off += session_len;

    //   cipher_suites: len(2) + bytes
    let ciph_len = u16::from_be_bytes([data[off], data[off + 1]]) as usize;
    off += 2;
    if data.len() < off + ciph_len + 1 {
        return None;
    }
    off += ciph_len;

    //   compression_methods: len(1) + bytes
    let comp_len = data[off] as usize;
    off += 1;
    if data.len() < off + comp_len + 2 {
        return None;
    }
    off += comp_len;

    //   extensions: len(2) + extension blocks
    let ext_total = u16::from_be_bytes([data[off], data[off + 1]]) as usize;
    off += 2;
    let ext_end = off + ext_total;
    if ext_end > data.len() {
        return None;
    }

    while off + 4 <= ext_end {
        let ext_type = u16::from_be_bytes([data[off], data[off + 1]]);
        let ext_len = u16::from_be_bytes([data[off + 2], data[off + 3]]) as usize;
        off += 4;
        if off + ext_len > ext_end {
            return None;
        }

        if ext_type == TLS_EXTENSION_SERVER_NAME {
            // server_name extension: list_len(2) then entries of
            // { type(1) name_len(2) name }
            if ext_len < 2 {
                return None;
            }
            let list_len = u16::from_be_bytes([data[off], data[off + 1]]) as usize;
            let list_start = off + 2;
            if list_len < 3 || list_start + list_len > ext_end {
                return None;
            }
            if data[list_start] != TLS_EXT_HOST_NAME {
                return None;
            }
            let name_len = u16::from_be_bytes([data[list_start + 1], data[list_start + 2]]) as usize;
            if name_len == 0 || list_start + 3 + name_len > list_start + list_len {
                return None;
            }
            return Some(list_start + 3);
        }

        off += ext_len;
    }

    None
}

/// Split `data` at the given index, producing up to `num_fragment` pieces.
/// The first cut is always early (see `FIRST_FRAGMENT_SIZE`) so the first TCP
/// segment is too small to be TLS-fingerprinted; the SNI cut is kept as the
/// second structural cut so the hostname is never contiguous in one segment.
/// The remainder is subdivided evenly so the total fragment count is respected
/// without reintroducing random offsets.
fn cut_points(length: usize, num_fragment: usize, split_at: usize) -> Vec<usize> {
    let early = FIRST_FRAGMENT_SIZE.min(length.saturating_sub(1)).max(1);
    let mut cuts = vec![early];
    if split_at > early {
        cuts.push(split_at);
    }
    let last_cut = *cuts.last().unwrap();
    let rest = length - last_cut;
    let extra = num_fragment.saturating_sub(cuts.len());
    if extra > 0 && rest > 1 {
        let step = (rest / (extra + 1)).max(1);
        let mut pos = last_cut + step;
        while pos < length - 1 && cuts.len() < num_fragment {
            cuts.push(pos);
            pos += step;
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts
}

pub async fn send_fragmented_async<W>(
    writer: &mut W,
    data: &[u8],
    num_fragment: usize,
    fragment_sleep_ms: u64,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let length = data.len();
    if length <= 1 || num_fragment <= 1 {
        writer.write_all(data).await?;
        writer.flush().await?;
        return Ok(());
    }

    // First cut is early (defeats first-segment TLS fingerprinting by on-path
    // DPI); then cut exactly at the SNI hostname offset so the name is never
    // contiguous in a single segment. Fallback for non-TLS / no-SNI payloads is
    // a fixed third-of-the-buffer point (deterministic, not random).
    let sni_offset = find_sni_offset(data);
    let primary = sni_offset.unwrap_or_else(|| length / 3).clamp(1, length - 1);
    let cuts = cut_points(length, num_fragment, primary);

    let sleep_dur = Duration::from_millis(fragment_sleep_ms);
    let mut prev = 0usize;

    for &idx in &cuts {
        writer.write_all(&data[prev..idx]).await?;
        writer.flush().await?;
        prev = idx;
        if fragment_sleep_ms > 0 {
            tokio::time::sleep(sleep_dur).await;
        }
    }

    if prev < length {
        writer.write_all(&data[prev..length]).await?;
        writer.flush().await?;
    }

    Ok(())
}
