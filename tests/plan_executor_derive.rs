use galvolite::plan_executor::{derive_spawn_graph, DeriveError, SpawnGraph};

const PLAN: &str = "\
- [ ] First step
- [ ] Second step
- [ ] Third step
";

const DIRECTIVE: &str = "\
# Directive: demo
Plan: .galvolite/plan.md

## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | full | — |
| p2 | 2, 3 | review | p1 |

## Notes
none
";

#[test]
fn happy_path_returns_phases_in_topo_order() {
    let g: SpawnGraph = derive_spawn_graph(PLAN, DIRECTIVE).unwrap();
    assert_eq!(g.phases.len(), 2);
    assert_eq!(g.phases[0].id, "p1");
    assert_eq!(g.phases[0].part_titles, vec![(1u32, "First step".to_string())]);
    assert_eq!(g.phases[1].id, "p2");
    assert_eq!(
        g.phases[1].part_titles,
        vec![(2u32, "Second step".into()), (3u32, "Third step".into())]
    );
}

#[test]
fn orphan_part_rejected() {
    let bad = "\
# Directive: x
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 7 | full | — |
## Notes
";
    let err = derive_spawn_graph(PLAN, bad).unwrap_err();
    assert!(matches!(err, DeriveError::OrphanParts(parts) if parts == vec![7u32]));
}

#[test]
fn cycle_rejected() {
    let bad = "\
# Directive: x
Plan: .galvolite/plan.md
## Phases
| ID | Parts | Autonomy | Depends on |
| --- | --- | --- | --- |
| p1 | 1 | full | p2 |
| p2 | 2 | full | p1 |
## Notes
";
    let err = derive_spawn_graph(PLAN, bad).unwrap_err();
    assert!(matches!(err, DeriveError::Directive(_)));
}
