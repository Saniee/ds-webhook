# ds-webhook (blocking) — DISCONTINUED

> [!WARNING]
> **This blocking version is discontinued and no longer maintained.**
> It gets no bug fixes or updates. Use the async (tokio) version on the
> [`tokio`](https://github.com/Saniee/ds-webhook/tree/tokio) branch instead.
>
> Known problems that will not be fixed here: the public methods are `async fn`
> but block internally (they panic inside a tokio runtime), most calls ignore
> HTTP error status, and `Embed.timestamp` is sent as a number instead of an
> ISO8601 string.

A (somewhat) feature complete discord webhook wrapper written in rust, for simple apps.

## Depends on these libraries:

reqwest <br>
serde
