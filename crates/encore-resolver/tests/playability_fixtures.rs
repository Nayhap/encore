//! Recorded answers (D37): what each client said about a song, a private,
//! an age-restricted, a removed video and a live stream, recorded signed
//! out with `encore-cli resolve <id> --record crates/encore-resolver/fixtures/playability`
//! (scrubbed by `playability::recordable`) and classified as Encore should.
//! Record new ones when YouTube changes its wording, and add them here.

use encore_resolver::video::playability::{Playability, Restriction, classify};

fn fixture(name: &str) -> serde_json::Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/playability").join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).unwrap()
}

#[test]
fn recorded_answers_are_classified() {
    use Playability as P;
    let cases: &[(&str, Playability)] = &[
        // Songs that play.
        ("tv_samsung-4D7u5KF7SP8", P::Ok),
        ("tv_samsung-52fNscjkST4", P::Ok),
        ("tv_samsung-fa5IWHDbftI", P::Ok),
        ("visionos-4D7u5KF7SP8", P::Ok),
        // `tv` without its page config: "The page needs to be reloaded".
        ("tv-4D7u5KF7SP8", P::ClientRefused),
        // A private video: "Please sign in" on TV and visionOS, with the
        // reason in `messages` (never a sign-out, D4).
        ("tv_samsung-yZIXLfi8CZQ", P::Restricted(Restriction::Private)),
        ("visionos-yZIXLfi8CZQ", P::Restricted(Restriction::Private)),
        ("web_embedded-yZIXLfi8CZQ", P::Restricted(Restriction::Private)),
        ("tv-yZIXLfi8CZQ", P::Restricted(Restriction::Private)),
        // Age-restricted.
        ("tv_samsung-SZJvDhaSDnc", P::AgeGated),
        ("visionos-SZJvDhaSDnc", P::AgeGated),
        ("web_embedded-SZJvDhaSDnc", P::AgeGated),
        ("tv-SZJvDhaSDnc", P::AgeGated),
        // Removed for a policy violation.
        ("tv_samsung-6SJNVb0GnPI", P::Restricted(Restriction::Removed)),
        ("visionos-6SJNVb0GnPI", P::Restricted(Restriction::Removed)),
        ("web_embedded-6SJNVb0GnPI", P::Restricted(Restriction::Removed)),
        ("tv-6SJNVb0GnPI", P::Restricted(Restriction::Removed)),
        // A live stream.
        ("tv_samsung-jfKfPfyJRdk", P::Restricted(Restriction::Live)),
        ("visionos-jfKfPfyJRdk", P::Restricted(Restriction::Live)),
        ("web_embedded-jfKfPfyJRdk", P::Restricted(Restriction::Live)),
        ("tv-jfKfPfyJRdk", P::Restricted(Restriction::Live)),
    ];
    let mut wrong = Vec::new();
    for (name, want) in cases {
        let got = classify(&fixture(name));
        if &got != want {
            wrong.push(format!("{name}: {got:?}, want {want:?}"));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    let on_disk = std::fs::read_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/playability")).unwrap().count();
    assert_eq!(on_disk, cases.len(), "every recorded answer has an expected classification here");
}
