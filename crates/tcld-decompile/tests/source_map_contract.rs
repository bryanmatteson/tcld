use std::path::PathBuf;
use tcld_decompile::decompiler::CommandIdentity;
use tcld_decompile::{AnalysisSnapshot, LoadInput, OriginalSourceStatus};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/tcld/tests/fixtures")
        .join(name)
}

fn repo_fixture(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

#[test]
fn source_map_preserves_exact_chars_groups_original_status_and_nested_paths() {
    let snapshot = AnalysisSnapshot::from_input(LoadInput::path(fixture("proc.tbc")))
        .expect("fixture snapshot loads");
    let map = &snapshot.source_map;

    assert!(
        map.entries().len() >= snapshot.decompile.commands.len(),
        "source map should cover rendered command spans"
    );
    let proc_a = map
        .entries()
        .iter()
        .find(|entry| entry.text == "proc a { x } {return \"$x : $x\"}")
        .expect("proc a map entry");
    assert_eq!(proc_a.source_lines, 0..1);
    assert_eq!(proc_a.source_chars, 0..31);
    let proc_b = map
        .entries()
        .iter()
        .find(|entry| entry.text == "proc b { x } {return \"$x > $x\"}")
        .expect("proc b map entry");
    assert_eq!(proc_b.source_lines, 2..3);
    assert_eq!(proc_b.source_chars, 33..64);
    let return_a = map
        .entries()
        .iter()
        .find(|entry| {
            entry.parent_command_group == proc_a.command_group()
                && entry.text == "return \"$x : $x\""
        })
        .expect("proc a return");
    let return_b = map
        .entries()
        .iter()
        .find(|entry| {
            entry.parent_command_group == proc_b.command_group()
                && entry.text == "return \"$x > $x\""
        })
        .expect("proc b return");
    assert_eq!(return_a.source_lines, proc_a.source_lines);
    assert_eq!(return_b.source_lines, proc_b.source_lines);
    assert!(return_a.source_chars.start > proc_a.source_chars.start);
    assert!(return_b.source_chars.start > proc_b.source_chars.start);
    assert_eq!(
        &snapshot.decompile.text[return_a.source_chars.clone()],
        return_a.text
    );
    assert_eq!(
        &snapshot.decompile.text[return_b.source_chars.clone()],
        return_b.text
    );
    assert!(matches!(
        return_a.identity,
        CommandIdentity::Synthetic { .. }
    ));
    assert!(matches!(
        return_b.identity,
        CommandIdentity::Synthetic { .. }
    ));
    assert_ne!(return_a.synthetic_key(), return_b.synthetic_key());
    let list_call = map
        .entries()
        .iter()
        .find(|entry| entry.text == "list [a TEST] [b TEST]")
        .expect("list call map entry");
    assert_eq!(list_call.source_lines, 4..5);
    assert_eq!(list_call.source_chars, 66..88);
    assert!(
        map.entries()
            .iter()
            .any(|entry| entry.command_path.components.len() > 1
                && entry.parent_command_group.is_some()),
        "nested proc-body spans should carry a stable nested command path and parent group"
    );
    assert!(
        map.entries().iter().any(|entry| matches!(
            entry.original_status,
            OriginalSourceStatus::Available | OriginalSourceStatus::Unavailable
        )),
        "original source availability must be explicit"
    );

    let entry = map
        .entries()
        .iter()
        .find(|entry| entry.command_group().is_some() && !entry.text.trim().is_empty())
        .expect("mapped command entry");
    let pc = entry.bytecode_range.start;
    assert!(
        map.source_for_bytecode(pc)
            .iter()
            .any(|candidate| candidate.command_path == entry.command_path)
    );
    assert!(
        map.entries_for_command_path(&entry.command_path)
            .iter()
            .any(|candidate| candidate.bytecode_range == entry.bytecode_range)
    );
    assert!(
        map.lookup_decompiled_position(proc_b.source_lines.start, 5)
            .iter()
            .any(|candidate| candidate.command_path == proc_b.command_path)
    );
    let original_span = proc_a
        .original_span
        .clone()
        .expect("path loads should preserve decoded original source span");
    assert!(
        map.lookup_original_offset(original_span.start)
            .iter()
            .any(|candidate| candidate.command_path == proc_a.command_path)
    );

    let spans = vec![
        (0..10, "set a 1".to_string(), 0..1),
        (10..25, "if {$x} {\n    puts hello\n}".to_string(), 1..4),
    ];
    let from_spans = tcld_decompile::SourceMap::from_spans(&spans);
    assert_eq!(from_spans.entries()[0].source_chars, 0..7);
    assert_eq!(
        from_spans.entries()[1].source_chars,
        8..(8 + "if {$x} {\n    puts hello\n}".len())
    );
    assert!(
        from_spans
            .lookup_decompiled_position(2, 4)
            .iter()
            .any(|entry| entry.bytecode_range == (10..25)),
        "line/character lookup should use the requested line, not entry start + character"
    );

    let raw_payload = std::fs::read_to_string(fixture("expr.tbc")).expect("payload fixture");
    let raw_snapshot =
        AnalysisSnapshot::from_input(LoadInput::raw_payload_text(raw_payload, "raw-expr.tbc"))
            .expect("raw snapshot loads");
    assert!(
        raw_snapshot
            .source_map
            .entries()
            .iter()
            .all(|entry| entry.original_span.is_none()
                && entry.original_status == OriginalSourceStatus::Unavailable),
        "raw payloads without original source must not fabricate original spans"
    );
}

#[test]
fn bcproc_source_map_exposes_nested_source_commands() {
    let snapshot = AnalysisSnapshot::from_input(LoadInput::path(repo_fixture(
        "testing/fixtures/public/public_complex_fixture.tbc",
    )))
    .expect("public fixture snapshot loads");
    let map = &snapshot.source_map;
    let root = map
        .entries()
        .iter()
        .find(|entry| entry.text.starts_with("proc fixture::startlog "))
        .expect("fixture::startlog root command");
    let root_group = root.command_group().expect("root command group");

    assert_eq!(root.command_path.components, vec![root_group]);
    assert_eq!(root.parent_command_group, None);
    assert!(root.source_lines.start < root.source_lines.end);

    let nested = ["variable baseDir", "global fixture_startlog", "set err "]
        .into_iter()
        .map(|snippet| {
            map.entries()
                .iter()
                .find(|entry| {
                    entry.parent_command_group == Some(root_group)
                        && entry.command_path.components.first() == Some(&root_group)
                        && entry.command_path.components.len() == 2
                        && entry.text.trim_start().starts_with(snippet)
                })
                .unwrap_or_else(|| panic!("missing nested command {snippet:?}"))
        })
        .collect::<Vec<_>>();

    for entry in nested {
        assert!(
            root.source_lines.start < entry.source_lines.start
                && entry.source_lines.end <= root.source_lines.end,
            "nested span should be inside the proc source span: root={:?} nested={:?} text={:?}",
            root.source_lines,
            entry.source_lines,
            entry.text
        );
        assert_eq!(entry.original_span, None);
        assert_eq!(entry.original_status, OriginalSourceStatus::Unavailable);
        assert!(
            map.entries_for_command_path(&entry.command_path)
                .iter()
                .any(|candidate| candidate.source_lines == entry.source_lines),
            "nested command path should be indexed"
        );
    }
}

#[test]
fn bcproc_source_map_exposes_commands_inside_compound_source_bodies() {
    let snapshot = AnalysisSnapshot::from_input(LoadInput::path(repo_fixture(
        "testing/fixtures/public/public_complex_fixture.tbc",
    )))
    .expect("public fixture snapshot loads");
    let map = &snapshot.source_map;
    let root = map
        .entries()
        .iter()
        .find(|entry| entry.text.starts_with("proc fixture::logLevelInit "))
        .expect("fixture::logLevelInit root command");
    let root_group = root.command_group().expect("root command group");

    for snippet in [
        "foreach el $debug_types",
        "set tvar \"FIXTURE_LOG_LEVEL_$el\"",
        "if {[info exists \"::env($tvar)\"]}",
        "set debug_level($el) $::env($tvar)",
        "fixture::setValue \"debug_level_$el\" $debug_level($el)",
        "set initialized 1",
    ] {
        let entry = map
            .entries()
            .iter()
            .find(|entry| {
                entry.parent_command_group == Some(root_group)
                    && entry.command_path.components.first() == Some(&root_group)
                    && entry.command_path.components.len() == 2
                    && entry.text.trim_start().starts_with(snippet)
            })
            .unwrap_or_else(|| panic!("missing nested command {snippet:?}"));
        assert!(
            root.source_lines.start < entry.source_lines.start
                && entry.source_lines.end <= root.source_lines.end,
            "nested span should be inside the proc source span: root={:?} nested={:?} text={:?}",
            root.source_lines,
            entry.source_lines,
            entry.text
        );
        assert_eq!(entry.original_span, None);
        assert_eq!(entry.original_status, OriginalSourceStatus::Unavailable);
        assert!(
            map.entries_for_command_path(&entry.command_path)
                .iter()
                .any(|candidate| candidate.source_lines == entry.source_lines),
            "nested command path should be indexed"
        );
    }
}

#[test]
fn bcproc_source_map_synthetic_identity_carries_proc_qname_and_offset() {
    let snapshot = AnalysisSnapshot::from_input(LoadInput::path(repo_fixture(
        "testing/fixtures/public/public_complex_fixture.tbc",
    )))
    .expect("public fixture snapshot loads");
    let map = &snapshot.source_map;

    // Pick a representative nested command inside fixture::logLevelInit's body.
    let entry = map
        .entries()
        .iter()
        .find(|entry| {
            entry
                .text
                .trim_start()
                .starts_with("foreach el $debug_types")
                && matches!(entry.identity, CommandIdentity::Synthetic { .. })
        })
        .expect("synthetic foreach span inside fixture::logLevelInit body");

    let CommandIdentity::Synthetic {
        proc_qname,
        body_byte_offset,
        body_byte_len,
        kind,
        name,
    } = &entry.identity
    else {
        panic!("expected synthetic identity, got {:?}", entry.identity);
    };
    let qname = proc_qname
        .as_deref()
        .expect("synthetic spans inside a bcproc body must be bound to the enclosing proc");
    assert_eq!(qname, "fixture::logLevelInit");
    assert!(*body_byte_len > 0);
    assert_eq!(*kind, tcld_syntax::TclCommandKind::Command);
    assert_eq!(name.as_deref(), Some("foreach"));

    // The synthetic index round-trips on the runtime-key shape.
    let hits = map.lookup_synthetic(qname, *body_byte_offset);
    assert!(
        hits.iter()
            .any(|hit| hit.source_lines == entry.source_lines),
        "lookup_synthetic should return the entry by (proc_qname, body_byte_offset)"
    );

    // A miss should be empty rather than returning unrelated entries.
    assert!(
        map.lookup_synthetic("does::not::exist", 0).is_empty(),
        "unknown synthetic key returns no hits"
    );
}
