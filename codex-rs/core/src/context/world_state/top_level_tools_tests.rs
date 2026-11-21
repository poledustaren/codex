//! Catalog diffs preserve unchanged definitions, including tools inside namespaces.

use super::*;
use crate::context::world_state::test_support::render_section_cases;
use crate::context_manager::updates::merge_world_state_updates;
use codex_code_mode::ExecDefinition;
use codex_models_manager::model_info::model_info_from_slug;
use codex_prompts::ResolvedModelMessages;
use pretty_assertions::assert_eq;
use serde_json::json;

fn declaration(name: &str) -> Value {
    json!({"type": "function", "name": name, "description": "Look up a value.",
        "parameters": {"type": "object", "properties": {}}})
}

fn item(tools: Vec<Value>) -> ResponseItem {
    ResponseItem::AdditionalTools {
        id: None,
        role: "developer".to_string(),
        tools: Arc::from(tools),
    }
}

fn namespace(name: &str, tools: Vec<Value>) -> Value {
    json!({"type": "namespace", "name": name, "description": "Tools.", "tools": tools})
}

fn exec_declaration(description: &str, grammar: &str) -> Value {
    json!({"type": "namespace", "name": "functions", "description": "",
        "tools": [{"type": "custom", "name": "exec", "description": description,
            "format": {"type": "grammar", "syntax": "lark", "definition": grammar}}]})
}

#[test]
fn snapshots_catalog_transitions() {
    use PreviousSectionState::Absent;
    use PreviousSectionState::Known;

    let search = json!({"type": "tool_search", "execution": "client",
        "description": "Search tools.",
        "parameters": {"type": "object", "properties": {}}});
    let web = json!({"type": "web_search", "external_web_access": true});
    let original = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![search.clone()],
        /*metrics*/ None,
    )
    .unwrap();
    let mut changed = search;
    changed["parameters"]["properties"] = json!({"query": {"type": "string"}});
    let updated = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![changed, web.clone()],
        /*metrics*/ None,
    )
    .unwrap();
    let only_web = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![web],
        /*metrics*/ None,
    )
    .unwrap();
    let empty = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        Vec::new(),
        /*metrics*/ None,
    )
    .unwrap();

    insta::assert_snapshot!(render_section_cases(&[
        (Absent, Known(&original)),
        (Known(&original), Known(&original)),
        (Known(&original), Known(&updated)),
        (Known(&updated), Known(&only_web)),
        (Known(&only_web), Known(&empty)),
        (Known(&empty), Known(&original)),
    ]));
}

#[test]
fn initial_catalog_preserves_order_and_unchanged_catalog_emits_nothing() {
    let definitions = vec![
        namespace("zebra", vec![declaration("lookup")]),
        namespace("alpha", vec![declaration("lookup")]),
        namespace("exec", vec![declaration("lookup")]),
    ];
    let state = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        definitions.clone(),
        /*metrics*/ None,
    )
    .unwrap();
    let (snapshot, updates) = state.render_diff(PreviousSectionState::Absent);
    assert_eq!(
        merge_world_state_updates(updates),
        vec![item(definitions.clone())]
    );
    let (_, updates) = state.render_diff(PreviousSectionState::Unknown);
    assert_eq!(
        merge_world_state_updates(updates),
        vec![item(definitions.clone())]
    );
    let snapshot = snapshot.unwrap();
    let saved = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(
        saved
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        vec![
            "alpha",
            "alpha.lookup",
            "exec",
            "exec.lookup",
            "zebra",
            "zebra.lookup"
        ]
    );
    let snapshot: TopLevelToolsSnapshot = serde_json::from_value(saved).unwrap();
    let (unchanged_snapshot, updates) = state.render_diff(PreviousSectionState::Known(&snapshot));
    assert_eq!(unchanged_snapshot, Some(snapshot.clone()));
    assert!(updates.is_empty());

    let reworded = TopLevelToolsState::new(
        ResolvedIncrementalToolMessages {
            tool_update_hint: "New wording".to_string(),
            removed_tools_header: "Removed".to_string(),
            ..Default::default()
        },
        definitions,
        /*metrics*/ None,
    )
    .unwrap();
    let (reworded_snapshot, updates) = reworded.render_diff(PreviousSectionState::Known(&snapshot));
    assert_eq!(reworded_snapshot, Some(snapshot));
    assert!(updates.is_empty());
}

#[test]
fn empty_catalog_is_persisted_and_can_add_tools_again() {
    let definitions = vec![namespace("functions", vec![declaration("lookup")])];
    let original = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        definitions.clone(),
        /*metrics*/ None,
    )
    .unwrap();
    let previous = original
        .render_diff(PreviousSectionState::Absent)
        .0
        .unwrap();
    let empty = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        Vec::new(),
        /*metrics*/ None,
    )
    .unwrap();
    let (snapshot, updates) = empty.render_diff(PreviousSectionState::Known(&previous));
    assert_eq!(
        snapshot,
        Some(TopLevelToolsSnapshot::ToolsOnly(BTreeMap::new()))
    );
    assert_eq!(
        merge_world_state_updates(updates),
        vec![ContextualUserFragment::into(RemovedTools {
            messages: Default::default(),
            namespaces: vec!["functions".to_string()],
            ..Default::default()
        })]
    );
    let (_, updates) = original.render_diff(PreviousSectionState::Known(&snapshot.unwrap()));
    assert_eq!(
        merge_world_state_updates(updates),
        vec![
            ContextualUserFragment::into(IncrementalToolsUpdate(
                ResolvedIncrementalToolMessages::default().tool_update_hint
            )),
            item(definitions),
        ]
    );
}

#[test]
fn namespace_diff_contains_only_changed_and_added_tools() {
    let original = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![
            namespace(
                "functions",
                vec![
                    declaration("changed"),
                    declaration("unchanged"),
                    declaration("removed"),
                ],
            ),
            namespace("other", vec![declaration("changed")]),
        ],
        /*metrics*/ None,
    )
    .unwrap();
    let snapshot = original
        .render_diff(PreviousSectionState::Absent)
        .0
        .unwrap();
    let mut changed = declaration("changed");
    changed["parameters"]["properties"] = json!({"query": {"type": "string"}});
    let added = declaration("added");
    let current = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![
            namespace(
                "functions",
                vec![changed.clone(), declaration("unchanged"), added.clone()],
            ),
            namespace("other", vec![declaration("changed")]),
        ],
        /*metrics*/ None,
    )
    .unwrap();
    let (snapshot, updates) = current.render_diff(PreviousSectionState::Known(&snapshot));
    let expected = namespace("functions", vec![changed, added]);
    assert_eq!(
        merge_world_state_updates(updates),
        vec![
            ContextualUserFragment::into(IncrementalToolsUpdate(
                ResolvedIncrementalToolMessages::default().tool_update_hint
            )),
            item(vec![expected]),
            ContextualUserFragment::into(RemovedTools {
                messages: Default::default(),
                tools: vec!["functions.removed".to_string()],
                ..Default::default()
            }),
        ]
    );
    assert!(
        current
            .render_diff(PreviousSectionState::Known(&snapshot.unwrap()))
            .1
            .is_empty()
    );
}

#[test]
fn new_namespaces_share_one_notice_before_their_declarations() {
    let original = namespace("original", vec![declaration("lookup")]);
    let state = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![original.clone()],
        /*metrics*/ None,
    )
    .unwrap();
    let previous = state.render_diff(PreviousSectionState::Absent).0.unwrap();
    let additions = vec![
        namespace("second", vec![declaration("lookup")]),
        namespace("third", vec![declaration("lookup")]),
    ];
    let mut state = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        std::iter::once(original).chain(additions.clone()).collect(),
        /*metrics*/ None,
    )
    .unwrap();
    let (snapshot, updates) = state.render_diff(PreviousSectionState::Known(&previous));
    let (prefix, remaining) = super::super::split_prefix_updates(updates);
    assert_eq!(
        prefix,
        vec![
            ContextualUserFragment::into(IncrementalToolsUpdate(
                ResolvedIncrementalToolMessages::default().tool_update_hint
            )),
            item(additions.clone()),
        ]
    );
    assert!(remaining.is_empty());
    state.messages.tool_update_hint.clear();
    let (_, updates) = state.render_diff(PreviousSectionState::Known(&previous));
    let (prefix, remaining) = super::super::split_prefix_updates(updates);
    assert_eq!(prefix, vec![item(additions)]);
    assert!(remaining.is_empty());
    assert!(
        state
            .render_diff(PreviousSectionState::Known(&snapshot.unwrap()))
            .1
            .is_empty()
    );
}

#[test_case::test_case(
    None,
    "Updated namespace guidance.",
    "Updated instructions for the functions namespace:\nUpdated namespace guidance."
)]
#[test_case::test_case(
    Some(("Catalog {name}: ", "Cleared {name}.")),
    "Literal {name} and {instructions}.",
    "Catalog functions: Literal {name} and {instructions}."
)]
#[test_case::test_case(
    Some(("", "Cleared {name}.")),
    "Runtime instructions.",
    "Runtime instructions."
)]
#[test_case::test_case(
    Some(("Catalog {name}: ", "Cleared {name}.")),
    "",
    "Cleared functions."
)]
#[test_case::test_case(Some(("Catalog {name}: ", "")), "", "")]
fn namespace_description_change_does_not_repeat_unchanged_tools(
    wording: Option<(&str, &str)>,
    instructions: &str,
    expected: &str,
) {
    let original = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![namespace("functions", vec![declaration("lookup")])],
        /*metrics*/ None,
    )
    .unwrap();
    let snapshot = original
        .render_diff(PreviousSectionState::Absent)
        .0
        .unwrap();
    let mut updated = namespace("functions", vec![declaration("lookup")]);
    updated["description"] = json!(instructions);
    let mut messages = ResolvedIncrementalToolMessages::default();
    if let Some((prefix, cleared_template)) = wording {
        messages.namespace_instructions_prefix = prefix.to_string();
        messages.namespace_instructions_cleared = cleared_template.to_string();
    }
    let current = TopLevelToolsState::new(messages, vec![updated], /*metrics*/ None).unwrap();
    let (snapshot, updates) = current.render_diff(PreviousSectionState::Known(&snapshot));
    assert_eq!(
        merge_world_state_updates(updates),
        if expected.is_empty() {
            Vec::new()
        } else {
            vec![ContextualUserFragment::into(
                DeveloperInstructions::new(expected).with_metadata(
                    codex_protocol::models::ContentItemMetadata::tool(Some(
                        codex_protocol::models::ContentItemNamespace::Functions,
                    )),
                ),
            )]
        }
    );
    assert!(
        current
            .render_diff(PreviousSectionState::Known(&snapshot.unwrap()))
            .1
            .is_empty()
    );
}

#[test]
fn tool_type_change_keeps_the_callable_name_available() {
    let original = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![namespace(
            "functions",
            vec![declaration("lookup"), declaration("removed")],
        )],
        /*metrics*/ None,
    )
    .unwrap();
    let snapshot = original
        .render_diff(PreviousSectionState::Absent)
        .0
        .unwrap();
    let custom = json!({"type": "custom", "name": "lookup", "description": "Look up a value.",
        "format": {"type": "text"}});
    let updated = namespace("functions", vec![custom]);
    let current = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![updated.clone()],
        /*metrics*/ None,
    )
    .unwrap();
    let (_, updates) = current.render_diff(PreviousSectionState::Known(&snapshot));
    assert_eq!(
        merge_world_state_updates(updates),
        vec![
            ContextualUserFragment::into(IncrementalToolsUpdate(
                ResolvedIncrementalToolMessages::default().tool_update_hint
            )),
            item(vec![updated]),
            ContextualUserFragment::into(RemovedTools {
                messages: Default::default(),
                tools: vec!["functions.removed".to_string()],
                ..Default::default()
            }),
        ]
    );
}

#[test]
fn namespace_removals_emit_one_notice() {
    let original = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![
            namespace(
                "functions",
                vec![
                    declaration("lookup"),
                    declaration("removed"),
                    declaration("also_removed"),
                ],
            ),
            namespace("function", vec![declaration("lookup")]),
            namespace("other", vec![declaration("lookup")]),
            json!({"type": "web_search"}),
        ],
        /*metrics*/ None,
    )
    .unwrap();
    let snapshot = original
        .render_diff(PreviousSectionState::Absent)
        .0
        .unwrap();
    let current = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![namespace("functions", vec![declaration("lookup")])],
        /*metrics*/ None,
    )
    .unwrap();
    let (_, updates) = current.render_diff(PreviousSectionState::Known(&snapshot));
    assert_eq!(
        merge_world_state_updates(updates),
        vec![ContextualUserFragment::into(RemovedTools {
            messages: Default::default(),
            namespaces: vec!["function".to_string(), "other".to_string()],
            tools: vec![
                "functions.also_removed".to_string(),
                "functions.removed".to_string(),
                "web_search".to_string(),
            ],
        })]
    );
}

#[test]
fn namespace_replaced_by_builtin_keeps_the_replacement_available() {
    let original = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![namespace("web_search", vec![declaration("lookup")])],
        /*metrics*/ None,
    )
    .unwrap();
    let snapshot = original
        .render_diff(PreviousSectionState::Absent)
        .0
        .unwrap();
    let replacement = json!({"type": "web_search"});
    let current = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![replacement.clone()],
        /*metrics*/ None,
    )
    .unwrap();
    let (_, updates) = current.render_diff(PreviousSectionState::Known(&snapshot));
    assert_eq!(
        merge_world_state_updates(updates),
        vec![
            item(vec![replacement]),
            ContextualUserFragment::into(RemovedTools {
                messages: Default::default(),
                tools: vec!["web_search.lookup".to_string()],
                ..Default::default()
            }),
        ],
    );
}

#[test]
fn hidden_exec_is_not_reintroduced_and_returning_exec_restores_full_catalog() {
    let original = exec_declaration("Shared MCP Types:", "grammar");
    let exec_definitions = Arc::new(vec![ExecDefinition {
        key: "Tool: tools.a".to_string(),
        description: "### a\ndeclare const tools: { a(): Promise<string> };".to_string(),
    }]);
    let current = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![original.clone()],
        /*metrics*/ None,
    )
    .unwrap()
    .with_incremental_exec_tools(Some(Arc::clone(&exec_definitions)));
    let (baseline, full) = current.render_diff(PreviousSectionState::Absent);
    assert_eq!(
        merge_world_state_updates(full),
        vec![ResponseItem::AdditionalTools {
            id: None,
            role: "developer".to_string(),
            tools: Arc::from(vec![original.clone()])
        },]
    );
    let hidden = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        Vec::new(),
        /*metrics*/ None,
    )
    .unwrap()
    .with_incremental_exec_tools(Some(exec_definitions));
    let (empty, updates) = hidden.render_diff(PreviousSectionState::Known(&baseline.unwrap()));
    assert_eq!(
        merge_world_state_updates(updates),
        vec![ContextualUserFragment::into(RemovedTools {
            namespaces: vec!["functions".to_string()],
            ..Default::default()
        })],
    );
    // Retain identities for removals on return, but require a fresh exec declaration.
    let restored: TopLevelToolsSnapshot =
        serde_json::from_value(serde_json::to_value(empty.unwrap()).unwrap()).unwrap();
    let namespace_instructions = ContextualUserFragment::into(
        DeveloperInstructions::new(
            "The functions namespace no longer has additional instructions.".to_string(),
        )
        .with_metadata(ContentItemMetadata::tool(Some(
            "functions".to_string().into(),
        ))),
    );
    assert_eq!(
        merge_world_state_updates(
            current
                .render_diff(PreviousSectionState::Known(&restored))
                .1
        ),
        vec![
            namespace_instructions.clone(),
            ResponseItem::AdditionalTools {
                id: None,
                role: "developer".to_string(),
                tools: Arc::from(vec![original.clone()])
            }
        ],
    );

    // A full restore also withdraws definitions removed while exec was hidden.
    let without_entries = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![original.clone()],
        /*metrics*/ None,
    )
    .unwrap()
    .with_incremental_exec_tools(Some(Arc::new(Vec::new())));
    assert_eq!(
        merge_world_state_updates(
            without_entries
                .render_diff(PreviousSectionState::Known(&restored))
                .1
        ),
        vec![
            namespace_instructions,
            ResponseItem::AdditionalTools {
                id: None,
                role: "developer".to_string(),
                tools: Arc::from(vec![original]),
            },
            ContextualUserFragment::into(RemovedExecDefinitions {
                header: ResolvedIncrementalToolMessages::default()
                    .exec_tools_removed_definitions_header,
                names: vec!["Tool: tools.a".to_string()],
            }),
        ],
    );
}

#[test_case::test_case(json!({}), None, None; "bundled")]
#[test_case::test_case(json!({"exec_tools_update_hint": null, "exec_tools_removed_definitions_header": null}), None, None; "null_fields")]
#[test_case::test_case(json!({"exec_tools_update_hint": "é".repeat(257), "exec_tools_removed_definitions_header": "Catalog removals:"}), None, Some("Catalog removals:".to_string()); "oversized_update_preserves_removal")]
#[test_case::test_case(json!({"exec_tools_update_hint": "Catalog update.", "exec_tools_removed_definitions_header": "é".repeat(257)}), Some("Catalog update.".to_string()), None; "oversized_removal_preserves_update")]
#[test_case::test_case(json!({"exec_tools_update_hint": "é".repeat(256), "exec_tools_removed_definitions_header": "🐈".repeat(128)}), Some("é".repeat(256)), Some("🐈".repeat(128)); "utf8_byte_limit")]
fn changed_and_added_entries_share_one_update_and_removed_entries_are_withdrawn(
    overrides: Value,
    expected_update_hint: Option<String>,
    expected_removal_header: Option<String>,
) {
    let definition = exec_declaration("Shared MCP Types:", "grammar");
    let original = TopLevelToolsState::new(
        /*messages*/ Default::default(),
        vec![definition.clone()],
        /*metrics*/ None,
    )
    .unwrap()
    .with_incremental_exec_tools(Some(Arc::new(vec![
        ExecDefinition {
            key: "Tool: tools.a".to_string(),
            description: "### a\ndeclare const tools: { a(): Promise<string> };".to_string(),
        },
        ExecDefinition {
            key: "Tool: tools.b".to_string(),
            description: "### b\ndeclare const tools: { b(): Promise<string> };".to_string(),
        },
    ])));
    let baseline = original
        .render_diff(PreviousSectionState::Absent)
        .0
        .unwrap();

    let mut model = model_info_from_slug("test-model");
    model.model_messages =
        Some(serde_json::from_value(json!({"tools": {"incremental_tools": overrides}})).unwrap());
    let current = TopLevelToolsState::new(
        ResolvedModelMessages::from_model(&model).incremental_tools(),
        vec![definition],
        /*metrics*/ None,
    )
    .unwrap()
    .with_incremental_exec_tools(Some(Arc::new(vec![
        ExecDefinition {
            key: "Tool: tools.a".to_string(),
            description:
                "### a\nChanged a.\ndeclare const tools: { a(value: number): Promise<number> };"
                    .to_string(),
        },
        ExecDefinition {
            key: "Tool: tools.c".to_string(),
            description: "### c\ndeclare const tools: { c(): Promise<boolean> };".to_string(),
        },
    ])));
    let (snapshot, updates) = current.render_diff(PreviousSectionState::Known(&baseline));
    let defaults = ResolvedIncrementalToolMessages::default();
    let expected_update_hint = expected_update_hint.unwrap_or(defaults.exec_tools_update_hint);
    let expected_removal_header =
        expected_removal_header.unwrap_or(defaults.exec_tools_removed_definitions_header);
    assert_eq!(
        merge_world_state_updates(updates),
        vec![
            ContextualUserFragment::into(IncrementalToolsUpdate(format!(
                "{expected_update_hint}\n\n## Namespace: functions\n### a\nChanged a.\ndeclare const tools: {{ a(value: number): Promise<number> }};\n\n### c\ndeclare const tools: {{ c(): Promise<boolean> }};",
            ))),
            ContextualUserFragment::into(RemovedExecDefinitions {
                header: expected_removal_header,
                names: vec!["Tool: tools.b".to_string()],
            }),
        ]
    );
    assert!(
        current
            .render_diff(PreviousSectionState::Known(&snapshot.unwrap()))
            .1
            .is_empty()
    );
}
