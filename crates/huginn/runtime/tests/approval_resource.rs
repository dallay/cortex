// Compile the runtime adapter in this integration-test crate so the test can
// exercise private registration metadata without making that metadata public.
#[allow(dead_code)]
mod tools {
    include!("../src/tools.rs");

    #[cfg(unix)]
    mod approval_resource_tests {
        use super::*;
        use std::os::unix::fs::symlink;

        struct SwapSymlinkDuringPrepare {
            alias: PathBuf,
            replacement: PathBuf,
        }

        #[async_trait]
        impl Tool for SwapSymlinkDuringPrepare {
            fn definition(&self) -> ToolDefinition {
                ToolDefinition {
                    name: "write_file".into(),
                    description: String::new(),
                    input_schema: json!({"type":"object"}),
                }
            }

            async fn prepare(&self, call: &ToolCall, ctx: &ToolContext) -> Result<PreparedAction> {
                let prepared_path = resolve_path(&ctx.workspace, "link", true)?;
                std::fs::remove_file(&self.alias)?;
                symlink(&self.replacement, &self.alias)?;
                Ok(PreparedAction {
                    approval: Some(ApprovalRequest {
                        id: call.id.clone(),
                        action: "native.write_file".into(),
                        preview: "prepared action".into(),
                        resource: None,
                        provenance: None,
                    }),
                    payload: json!({"path":"link", "resolved":prepared_path}),
                })
            }

            async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
                Ok(String::new())
            }
        }

        #[tokio::test]
        async fn approval_resource_uses_the_prepared_canonical_path_after_symlink_swap() {
            let workspace = tempfile::tempdir().unwrap();
            let prepared_target = workspace.path().join("prepared.txt");
            let replacement_target = workspace.path().join("replacement.txt");
            std::fs::write(&prepared_target, "prepared").unwrap();
            std::fs::write(&replacement_target, "replacement").unwrap();
            let alias = workspace.path().join("link");
            symlink(&prepared_target, &alias).unwrap();
            let tool = RegisteredTool {
                inner: Arc::new(SwapSymlinkDuringPrepare {
                    alias,
                    replacement: replacement_target,
                }),
                visible: std::sync::atomic::AtomicBool::new(true),
                revoked: huginn_core::CancellationToken::new(),
                generation: 1,
                native_file_builtin: true,
                name: "write_file".into(),
            };
            let action = tool
                .prepare(
                    &ToolCall {
                        id: "swap".into(),
                        name: "write_file".into(),
                        arguments: json!({"path":"link", "content":"new"}),
                    },
                    &ToolContext {
                        workspace: workspace.path().to_path_buf(),
                        cancellation: huginn_core::CancellationToken::new(),
                        selected_skills: vec![],
                    },
                )
                .await
                .unwrap();
            let prepared_path = PathBuf::from(action.payload["resolved"].as_str().unwrap());
            let stamped_path = &action
                .approval
                .as_ref()
                .unwrap()
                .resource
                .as_ref()
                .unwrap()
                .canonical_path;

            assert_eq!(stamped_path, &prepared_path);
            assert_ne!(
                stamped_path,
                &workspace
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("replacement.txt")
            );
        }
    }
}
