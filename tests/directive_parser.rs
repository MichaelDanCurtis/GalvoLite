use galvolite::directive::{parse, Autonomy, ParseError};

const SAMPLE: &str = r#"# Directive: Sub-plan 7

Plan: ./plan.md

## Phases

| ID | Parts | Autonomy | Depends on |
|----|-------|----------|------------|
| P1 | 1     | full     | —          |
| P2 | 2, 3  | review   | P1         |
| P3 | 4     | manual   | P1, P2     |

## Escalation rules

- On schema change → ask_operator
- On executor stuck >3 cycles → notify_operator(level=warn)

## Notes

Free-form context for the orchestrator.
"#;

#[test]
fn parses_full_directive() {
    let d = parse(SAMPLE).unwrap();
    assert_eq!(d.name, "Sub-plan 7");
    assert_eq!(d.plan_path.to_str().unwrap(), "./plan.md");

    assert_eq!(d.phases.len(), 3);
    assert_eq!(d.phases[0].id, "P1");
    assert_eq!(d.phases[0].parts, vec![1]);
    assert!(matches!(d.phases[0].autonomy, Autonomy::Full));
    assert!(d.phases[0].depends_on.is_empty());

    assert_eq!(d.phases[1].parts, vec![2, 3]);
    assert!(matches!(d.phases[1].autonomy, Autonomy::Review));
    assert_eq!(d.phases[1].depends_on, vec!["P1"]);

    assert_eq!(d.phases[2].depends_on, vec!["P1", "P2"]);

    assert_eq!(d.escalation_rules.len(), 2);
    assert!(d.escalation_rules[0].trigger.contains("schema change"));
    assert!(d.escalation_rules[0].action.contains("ask_operator"));

    assert!(d.notes.starts_with("Free-form"));
}

#[test]
fn rejects_circular_dependencies() {
    let bad = "# Directive: D\n\nPlan: p\n\n## Phases\n\n| ID | Parts | Autonomy | Depends on |\n|----|-------|----------|------------|\n| P1 | 1     | full     | P2         |\n| P2 | 2     | full     | P1         |\n";
    assert!(matches!(parse(bad), Err(ParseError::CircularDep(_))));
}

#[test]
fn rejects_unknown_autonomy() {
    let bad = SAMPLE.replace("full", "yolo");
    assert!(matches!(parse(&bad), Err(ParseError::BadAutonomy(_))));
}

#[test]
fn rejects_dependency_on_undefined_phase() {
    let bad = SAMPLE.replace("P1, P2", "P1, P9");
    assert!(matches!(parse(&bad), Err(ParseError::UndefinedDep { .. })));
}

#[test]
fn rejects_empty_title() {
    let bad = SAMPLE.replace("Sub-plan 7", "");
    assert!(matches!(parse(&bad), Err(ParseError::MissingSection(_))));
}

#[test]
fn rejects_empty_phases_table() {
    let bad = "# Directive: D\n\nPlan: ./p\n\n## Phases\n\n| ID | Parts | Autonomy | Depends on |\n|----|-------|----------|------------|\n\n## Notes\n\n";
    assert!(matches!(parse(bad), Err(ParseError::MissingSection(_))));
}
