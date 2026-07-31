use std::cmp::min;
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use rand::seq::SliceRandom;

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

    let max_splits = min(num_fragment - 1, length - 2);
    if max_splits == 0 {
        writer.write_all(data).await?;
        writer.flush().await?;
        return Ok(());
    }

    let indices: Vec<usize> = {
        let mut rng = rand::thread_rng();
        let mut possible_indices: Vec<usize> = (1..length - 1).collect();
        possible_indices.shuffle(&mut rng);
        let mut ind: Vec<usize> = possible_indices.into_iter().take(max_splits).collect();
        ind.sort_unstable();
        ind
    };

    let sleep_dur = Duration::from_millis(fragment_sleep_ms);
    let mut prev = 0;

    for &idx in &indices {
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
