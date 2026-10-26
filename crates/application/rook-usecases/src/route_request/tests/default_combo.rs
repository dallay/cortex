use super::*;

async fn run(
    usecase: &RouteRequest,
    mut req: CompletionRequest,
    streaming: bool,
) -> CortexResult<()> {
    req.stream = streaming;
    if streaming {
        let chunks: Vec<_> = usecase.execute_stream(req).await?.collect().await;
        assert!(!chunks.is_empty());
        for chunk in chunks {
            chunk?;
        }
    } else {
        usecase.execute(req).await?;
    }
    Ok(())
}

#[tokio::test]
async fn default_combo_uses_physical_steps_in_both_paths() {
    for streaming in [false, true] {
        let combo = harness_combo();
        let id = combo.id;
        let mut harness = combo_harness(combo, vec![]);
        harness.usecase = harness.usecase.with_default_combo(Some(id));
        run(
            &harness.usecase,
            combo_req("requested-model", rook_core::ApiKeyRestrictions::default()),
            streaming,
        )
        .await
        .unwrap();
        assert_eq!(harness.seen_models.lock().unwrap().as_slice(), ["gpt-4o"]);
        let audit = harness.audit.entries.lock().unwrap();
        assert_eq!(audit[0].combo_id, Some(id));
    }
}

#[tokio::test]
async fn explicit_header_and_virtual_alias_take_precedence_over_default() {
    for streaming in [false, true] {
        let combo = harness_combo();
        let id = combo.id;
        let mut harness = combo_harness(combo, vec![("rook/coding", id)]);
        harness.usecase = harness.usecase.with_default_combo(Some(ComboId::new()));
        let mut req = combo_req("requested-model", rook_core::ApiKeyRestrictions::default());
        req.metadata.combo_id = Some(id);
        run(&harness.usecase, req, streaming).await.unwrap();
        run(
            &harness.usecase,
            combo_req("rook/coding", rook_core::ApiKeyRestrictions::default()),
            streaming,
        )
        .await
        .unwrap();
        let error = run(
            &harness.usecase,
            combo_req("rook/unknown", rook_core::ApiKeyRestrictions::default()),
            streaming,
        )
        .await
        .unwrap_err();
        assert!(error.is_not_found(), "{error}");
        assert_eq!(harness.seen_models.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn default_combo_preflights_all_step_restrictions_before_any_upstream() {
    for streaming in [false, true] {
        for restrict_models in [false, true] {
            let combo = Combo::new(
                "restricted fallback".to_string(),
                ComboStrategy::Priority,
                vec![
                    step("openai", "gpt-4o", 1),
                    step("blocked", "blocked-model", 2),
                ],
            );
            let id = combo.id;
            let mut harness = combo_harness(combo, vec![]);
            harness.usecase = harness.usecase.with_default_combo(Some(id));
            let restrictions = rook_core::ApiKeyRestrictions {
                allowed_models: if restrict_models {
                    vec![ModelId::new("requested-model"), ModelId::new("gpt-4o")]
                } else {
                    vec![]
                },
                allowed_providers: if restrict_models {
                    vec![]
                } else {
                    vec![ProviderId::new("openai")]
                },
            };
            let error = run(
                &harness.usecase,
                combo_req("requested-model", restrictions),
                streaming,
            )
            .await
            .unwrap_err();
            assert!(error.is_forbidden(), "{error}");
            assert!(harness.seen_models.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn absent_default_keeps_standard_routing_in_both_paths() {
    for streaming in [false, true] {
        let harness = combo_harness(harness_combo(), vec![]);
        run(
            &harness.usecase,
            combo_req("requested-model", rook_core::ApiKeyRestrictions::default()),
            streaming,
        )
        .await
        .unwrap();
        assert_eq!(
            harness.seen_models.lock().unwrap().as_slice(),
            ["requested-model"]
        );
    }
}
