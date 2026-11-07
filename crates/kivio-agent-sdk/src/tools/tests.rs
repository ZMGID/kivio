use super::*;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct Args {
    text: String,
}

struct Count;

impl StaticTool for Count {
    type Args = Args;
    type Output = usize;
    type Error = &'static str;
    type HostData = String;

    fn execute<'a>(
        &'a self,
        args: Args,
    ) -> ToolFuture<'a, Result<ToolOutput<String, usize>, Self::Error>> {
        Box::pin(async move {
            if args.text.is_empty() {
                return Err("empty text");
            }
            let count = args.text.chars().count();
            Ok(ToolOutput {
                content: format!("{count} characters"),
                structured_content: Some(count),
                host_data: "private display data".into(),
                follow_up_messages: Vec::new(),
                is_error: false,
            })
        })
    }
}

#[tokio::test]
async fn typed_tool_keeps_model_structured_and_host_output_separate() {
    let output = call_typed(&Count, serde_json::json!({"text":"你好"}))
        .await
        .unwrap();
    assert_eq!(output.content, "2 characters");
    assert_eq!(output.structured_content, Some(serde_json::json!(2)));
    assert_eq!(output.host_data, "private display data");
    assert!(matches!(
        call_typed(&Count, serde_json::json!({"text":12})).await,
        Err(ToolError::InvalidArguments(_))
    ));
    assert_eq!(
        call_typed(&Count, serde_json::json!({"text":""}))
            .await
            .unwrap_err(),
        ToolError::Execution("empty text".into())
    );
}

struct Unserializable;
impl Serialize for Unserializable {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("cannot encode this output"))
    }
}

struct BadOutput;
impl StaticTool for BadOutput {
    type Args = Args;
    type Output = Unserializable;
    type Error = std::convert::Infallible;
    type HostData = ();

    fn execute<'a>(
        &'a self,
        _: Args,
    ) -> ToolFuture<'a, Result<ToolOutput<(), Unserializable>, Self::Error>> {
        Box::pin(async {
            let mut output = ToolOutput::text("must not be reported as success");
            output.structured_content = Some(Unserializable);
            Ok(output)
        })
    }
}

#[tokio::test]
async fn output_encoding_failure_is_an_error() {
    assert!(
        matches!(call_typed(&BadOutput, serde_json::json!({"text":"hello"})).await,
        Err(ToolError::OutputEncoding(message)) if message.contains("cannot encode"))
    );
}
