use retriever_protocol::{Response, VERSION};
fn main() -> anyhow::Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    while let Some(request) = retriever_protocol::read_frame(&mut input)? {
        let framed = request.body["_framed_response"].as_bool().unwrap_or(false);
        let agent_state=std::env::var_os("RETRIEVER_AGENT_STATE").map(std::path::PathBuf::from).unwrap_or_else(||retriever_engine::paths::expand("~/.retriever-agent-candidate"));
        let result = retriever_engine::agent::dispatch(&request.action, &request.body, &[agent_state]);
        let response = match result {
            Ok(data) => Response {
                version: VERSION,
                id: request.id,
                ok: true,
                data: Some(data),
                error: None,
                fragment: None,
            },
            Err(e) => Response {
                version: VERSION,
                id: request.id,
                ok: false,
                data: None,
                error: Some(e.to_string()),
                fragment: None,
            },
        };
        if framed {
            retriever_protocol::write_response(&mut output, &response)?;
        } else {
            retriever_protocol::write_frame(&mut output, &response)?;
        }
    }
    Ok(())
}
