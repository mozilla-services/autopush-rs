use redis::{aio::ConnectionLike, cmd, pipe, AsyncCommands, Pipeline, RedisError, ToRedisArgs};

/// Async version of [redis::transaction]
///
/// Note that transaction usage is problematic when multiplexing is utilized on the passed in async
/// Connection, as this would cause the multiplexed commands to interleave with the
/// transaction's. The transaction's Connection should not be shared.
pub async fn transaction<
    C: ConnectionLike + AsyncCommands,
    K: ToRedisArgs,
    T,
    R: FnOnce() -> E,
    F: AsyncFnMut(&mut C, &mut Pipeline) -> Result<Option<T>, E>,
    E: From<RedisError>,
>(
    con: &mut C,
    keys: &[K],
    retries: usize,
    retry_err: R,
    func: F,
) -> Result<T, E> {
    let mut func = func;
    for _ in 0..retries {
        cmd("WATCH").arg(keys).exec_async(con).await?;
        let mut p = pipe();
        let response: Option<T> = func(con, p.atomic()).await?;
        match response {
            None => {
                continue;
            }
            Some(response) => {
                // make sure no watch is left in the connection, even if
                // someone forgot to use the pipeline.
                cmd("UNWATCH").exec_async(con).await?;
                return Ok(response);
            }
        }
    }
    Err(retry_err())
}
