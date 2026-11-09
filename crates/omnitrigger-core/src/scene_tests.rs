use crate::*;

fn scope(identity: ProcessIdentity, allow: &[&str], deny: &[&str]) -> ProcessScope {
    ProcessScope {
        identity,
        allow: allow.iter().map(|s| s.to_string()).collect(),
        deny: deny.iter().map(|s| s.to_string()).collect(),
    }
}

fn exe(allow: &[&str], deny: &[&str]) -> ProcessScope {
    scope(ProcessIdentity::Executable, allow, deny)
}

/// 交集必须与“两侧同时命中”逐点一致，包括未知进程与各窗口角色。
#[test]
fn intersection_matches_exactly_when_both_scopes_match() {
    let names = [
        "chrome", "msedge", "code", "desktop", "taskbar", "explorer", "other",
    ];
    let roles = [
        WindowRole::Unknown,
        WindowRole::Application,
        WindowRole::Desktop,
        WindowRole::Taskbar,
    ];
    let scopes = [
        ProcessScope::default(),
        scope(ProcessIdentity::ShellContext, &[], &[]),
        exe(&["chrome", "msedge"], &[]),
        exe(&["Chrome.exe", "code"], &["msedge"]),
        exe(&[], &["code"]),
        exe(&[], &["chrome"]),
        exe(&["msedge"], &["chrome"]),
        scope(ProcessIdentity::ShellContext, &["desktop", "explorer"], &[]),
        scope(ProcessIdentity::ShellContext, &[], &["taskbar"]),
    ];
    // 匹配只比较规范名，与注册时一致。
    let normalized = |scope: &ProcessScope| {
        let mut scope = scope.clone();
        scope.normalize().map(|_| scope)
    };
    for a in &scopes {
        for b in &scopes {
            match a.intersect(b) {
                Ok(both) => {
                    let (left, right) = (normalized(a).unwrap(), normalized(b).unwrap());
                    let both = normalized(&both);
                    for role in roles {
                        for process in names.iter().map(Some).chain([None]) {
                            let process = process.copied();
                            let expected = left.matches_context(process, role)
                                && right.matches_context(process, role);
                            match &both {
                                Ok(both) => assert_eq!(
                                    both.matches_context(process, role),
                                    expected,
                                    "{a:?} ∩ {b:?} at {process:?}/{role:?}"
                                ),
                                // 白名单全被排除时注册报错，此时两侧本就不会同时命中。
                                Err(ProcessScopeError::EmptyRange) => assert!(!expected),
                                Err(error) => panic!("{error}"),
                            }
                        }
                    }
                }
                Err(ScopeIntersectionError::IdentityMismatch) => {
                    assert!(!a.is_global() && !b.is_global() && a.identity != b.identity);
                }
                Err(ScopeIntersectionError::DisjointAllow) => {
                    assert!(!a.allow.is_empty() && !b.allow.is_empty());
                }
            }
        }
    }
}

#[test]
fn intersection_keeps_left_spelling_and_merges_deny_lists() {
    let both = exe(&["Chrome.exe", "code"], &["game"])
        .intersect(&exe(&["chrome", "msedge"], &["GAME.exe", "steam"]))
        .unwrap();
    assert_eq!(both, exe(&["Chrome.exe"], &["game", "steam"]));
    assert_eq!(
        exe(&["code"], &[]).intersect(&exe(&["chrome"], &[])),
        Err(ScopeIntersectionError::DisjointAllow)
    );
}

#[test]
fn scene_intersection_reports_the_conflicting_dimension() {
    let browser = Scene {
        foreground: exe(&["chrome", "msedge"], &[]),
        pointer: exe(&["chrome", "msedge"], &[]),
    };
    let edge_pointer = Scene {
        foreground: ProcessScope::default(),
        pointer: exe(&["msedge"], &[]),
    };
    assert_eq!(
        browser.intersect(&edge_pointer).unwrap(),
        Scene {
            foreground: exe(&["chrome", "msedge"], &[]),
            pointer: exe(&["msedge"], &[]),
        }
    );
    let shell = Scene {
        foreground: ProcessScope::default(),
        pointer: scope(ProcessIdentity::ShellContext, &["desktop"], &[]),
    };
    assert_eq!(
        browser.intersect(&shell),
        Err(SceneConflict {
            dimension: SceneDimension::Pointer,
            reason: ScopeIntersectionError::IdentityMismatch,
        })
    );
    assert!(Scene::default().is_unconstrained());
    assert!(!browser.is_unconstrained());
}

#[test]
fn scene_round_trips_through_trigger_spec_context_fields() {
    let mut spec = TriggerSpec {
        pointer_scope: Default::default(),
        wheel: Default::default(),
        keyboard: Default::default(),
        process_scope: Default::default(),
        id: 1,
        held: vec![Button::Key(20)],
        activation: Activation::Press(Button::Key(75)),
        conditions: vec![],
    };
    let scene = Scene {
        foreground: exe(&["code"], &[]),
        pointer: exe(&[], &["game"]),
    };
    scene.clone().apply_to(&mut spec);
    assert_eq!(spec.process_scope, scene.foreground);
    assert_eq!(spec.pointer_scope, scene.pointer);
    assert_eq!(Scene::of(&spec), scene);
    assert_eq!(
        exe(&["bad/path"], &[]).validate(),
        Err(ProcessScopeError::InvalidName {
            name: "bad/path".into()
        })
    );
}
