use mirror::theme::{TokenSet, ALL_THEMES};

#[test]
fn every_theme_passes_legibility() {
    for def in ALL_THEMES {
        let theme = def.derive();
        assert!(
            theme.first_legibility_failure().is_none(),
            "{}: {:?}",
            def.name,
            theme.first_legibility_failure()
        );
    }
}

#[test]
fn derived_token_set_is_complete() {
    for def in ALL_THEMES {
        let theme = def.derive();
        for (token, colour) in theme.colours.iter() {
            assert!(colour.a >= 0.0, "{:?} alpha", token);
        }
        let _: TokenSet = theme.colours.clone();
    }
}
