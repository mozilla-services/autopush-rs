use redis::{
    aio::ConnectionLike, cmd, pipe, AsyncCommands, ErrorKind, Pipeline, RedisError, ToRedisArgs,
};

const MAX_TRANSACTION_LOOP: u8 = 10;

/// Async version of [redis::transaction]
///
/// Note that transaction usage is problematic when multiplexing is utilized on the passed in async
/// Connection, as this would cause the multiplexed commands to interleave with the
/// transaction's. The transaction's Connection should not be shared.
pub async fn transaction<
    C: ConnectionLike + AsyncCommands,
    K: ToRedisArgs,
    T,
    F: AsyncFnMut(&mut C, &mut Pipeline) -> Result<Option<T>, E>,
    E: From<RedisError>,
>(
    con: &mut C,
    keys: &[K],
    func: F,
) -> Result<T, E> {
    let mut func = func;
    for _i in [0..MAX_TRANSACTION_LOOP] {
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

    // The transaction failed, so return an error.
    // We're using `Client` here because it's the closest that I could think of since
    // it implies an error BEFORE the script was executed. An alternate might be
    // `ExecAbortError` but that meant that the script failed in process.)
    Err(RedisError::from((ErrorKind::ClientError, "Could not complete transaction")).into())
}
