//! The solver corpus (D37): every player and variant of yt-dlp-ejs's own
//! tests (`src/yt/solver/test/tests.ts` at the ejs commit in
//! `upstream.toml`) and yt-dlp's `test/test_jsc/test_ejs_integration.py`,
//! solved by the embedded engine cold and from the preprocessed player.
//!
//! The players are YouTube's code, so they are not committed: they are
//! fetched once from `www.youtube.com/s/player/…` into
//! `$ENCORE_EJS_PLAYERS` (default: Cargo's test tmp dir) and read from
//! there afterwards, so later runs are offline.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use encore_resolver::jsc::quickjs::{Limits, QuickJsSolver};
use encore_resolver::jsc::{ChallengeRequest, ChallengeType, PlayerScript, Solver};

struct Case {
    player: &'static str,
    variants: &'static [&'static str],
    n: &'static [(&'static str, &'static str)],
    sig: &'static [(&'static str, &'static str)],
}

/// `players` in tests.ts (yt-dlp's `Variant` also has es5 and house, which
/// no case uses).
const VARIANTS: &[(&str, &str)] = &[
    ("main", "player_ias.vflset/en_US/base.js"),
    ("tcc", "player_ias_tcc.vflset/en_US/base.js"),
    ("tce", "player_ias_tce.vflset/en_US/base.js"),
    ("es6", "player_es6.vflset/en_US/base.js"),
    ("tv", "tv-player-ias.vflset/tv-player-ias.js"),
    ("tv_es6", "tv-player-es6.vflset/tv-player-es6.js"),
    ("phone", "player-plasma-ias-phone-en_US.vflset/base.js"),
    ("es6_tcc", "player_es6_tcc.vflset/en_US/base.js"),
    ("es6_tce", "player_es6_tce.vflset/en_US/base.js"),
];

/// tests.ts (every variant), then the cases yt-dlp adds (`main` or `tv`).
#[rustfmt::skip]
const CASES: &[Case] = &[
    Case { player: "74edf1a3", variants: &["main", "tcc", "tce", "es6", "tv", "tv_es6", "phone", "es6_tcc", "es6_tce"], n: &[("IlLiA21ny7gqA2m4p37", "9nRTxrbM1f0yHg"), ("eabGFpsUKuWHXGh6FR4", "izmYqDEY6kl7Sg"), ("eabGF/ps%UK=uWHXGh6FR4", "LACmqlhaBpiPlgE-a")], sig: &[("NJAJEij0EwRgIhAI0KExTgjfPk-MPM9MAdzyyPRt=BM8-XO5tm5hlMCSVpAiEAv7eP3CURqZNSPow8BXXAoazVoXgeMP7gH9BdylHCwgw=gwzz", "NJAJEij0EwRgIhAI0KExTgjfPk-MPM9MAdzyyPRt=BM8-XO5tm5hzMCSVpAiEAv7eP3CURqZNSPow8BXXAoazVoXgeMP7gH9BdylHCwgw=gwzl"), ("\x00\x01\x02%\x03\x04\x05\x06\x07\x08\x09\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19 !\"#$%&'()0123456789@ABCDEFGHI", "\x00\x01\x02%\x03\x04\x05\x06\x07\x08\x09\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19 !\"#$%&'()0123456789@ABIDEFGHC")] },
    Case { player: "901741ab", variants: &["main", "tcc", "tce", "es6", "tv", "tv_es6", "phone", "es6_tcc", "es6_tce"], n: &[("BQoJvGBkC2nj1ZZLK-", "UMPovvBZRh-sjb")], sig: &[("NJAJEij0EwRgIhAI0KExTgjfPk-MPM9MAdzyyPRt=BM8-XO5tm5hlMCSVpAiEAv7eP3CURqZNSPow8BXXAoazVoXgeMP7gH9BdylHCwgw=gwzz", "wgwCHlydB9Hg7PMegXoVzaoAXXB8woPSNZqRUC3Pe7vAEiApVSCMlhwmt5ON-8MB=5RPyyzdAM9MPM-kPfjgTxEK0IAhIgRwE0jiEJA")] },
    Case { player: "e7573094", variants: &["main", "tcc", "tce", "es6", "tv", "tv_es6", "phone", "es6_tcc", "es6_tce"], n: &[("IlLiA21ny7gqA2m4p37", "3KuQ3235dojTSjo4")], sig: &[("NJAJEij0EwRgIhAI0KExTgjfPk-MPM9MAdzyyPRt=BM8-XO5tm5hlMCSVpAiEAv7eP3CURqZNSPow8BXXAoazVoXgeMP7gH9BdylHCwgw=gwzz", "yEij0EwRgIhAI0KExTgjfPk-MPM9MAdzyNPRt=BM8-XO5tm5hlMCSVNAiEAvpeP3CURqZJSPow8BXXAoazVoXgeMP7gH9BdylHCwgw=g")] },
    Case { player: "9fcf08e8", variants: &["main", "tcc", "tce", "es6", "tv", "tv_es6", "phone", "es6_tcc", "es6_tce"], n: &[("4JRSxxN0E_fLVnxmd", "xhAoMP8xPcR5wg")], sig: &[("\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghij", "jihgfedcba`_^]\\[ZYXWVUTSRQPONMLKJIHGFEDCBA@?>=<;:9876543210/.-,+*)('&%$#\"! \x1f\x1e\x1d\x1c\x1b\x1a\x19\x18\x17\x16\x15\x14\x13\x12\x11\x10\x0f\x0e\x0d\x0c\x0b\x03\x09\x08\x07\x06\x05\x04\x0a")] },
    Case { player: "21cd2156", variants: &["main", "tcc", "tce", "es6", "tv", "tv_es6", "phone", "es6_tcc", "es6_tce"], n: &[("16EF3jx-Mr_TLuGH", "OQJDc7IrlWCkfg")], sig: &[("\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghij", "\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCD\x00FGHIJKjMNOPQRSTUVWXYZ[\\]^_`abcdefghiL")] },
    Case { player: "76ad2fe8", variants: &["main", "tcc", "tce", "es6", "tv", "tv_es6", "phone", "es6_tcc", "es6_tce"], n: &[("V8pdMGD0Sz_M0DRT", "rBnTGCWGFpwByA")], sig: &[("\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghij", "Ffedcba`_^gj[ZYXWVUTSRQPONMLKJIHG,EDCBA@?>=<;:98\x136543210/.-]+*)('&%$#\"! \x1f\x1e\x1d\x1c\x1b\x1a\x19\x18\x17\x16\x15\x14\x0c\x12\x11\x10\x0f\x0e\x0d\x00\x0b\x0a\x09\x08\x07\x06\x05\x04\x03\x02\x017")] },
    Case { player: "631d3938", variants: &["main", "tcc", "tce", "es6", "tv", "tv_es6", "phone", "es6_tcc", "es6_tce"], n: &[("KBx1qz7jMhxELa8c", "ttPvh7WIptsgSw")], sig: &[("\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdef", "\x19\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x00\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abc")] },
    Case { player: "9fcf08e8", variants: &["main"], n: &[("Dyc5ALyWiO0VqwCiT", "H2PLmmAmJsYjKA")], sig: &[] },
    Case { player: "21cd2156", variants: &["main"], n: &[("CiOxDbW1WEE8Ti4w", "ZcBE4klItiC4rQ")], sig: &[] },
    Case { player: "5e55da5a", variants: &["tv"], n: &[("FgTvzyq4jKv482R7", "l26nyYSotkzDxg")], sig: &[("\x00\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b\x0c\x0d\x0e\x0f\x10\x11\x12\x13\x14\x15\x16\x17\x18\x19\x1a\x1b\x1c\x1d\x1e\x1f !\"#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`abcdefghij", "Ffedcba`_^gj[ZYXWVUTSRQPONMLKJIHG,EDCBA@?>=<;:98\x136543210/.-]+*)('&%$#\"! \x1f\x1e\x1d\x1c\x1b\x1a\x19\x18\x17\x16\x15\x14\x0c\x12\x11\x10\x0f\x0e\x0d\x00\x0b\x0a\x09\x08\x07\x06\x05\x04\x03\x02\x017")] },
];

fn cache_dir() -> PathBuf {
    std::env::var_os("ENCORE_EJS_PLAYERS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ejs-players"))
}

async fn player(http: &reqwest::Client, id: &str, variant: &str) -> String {
    let dir = cache_dir();
    let file = dir.join(format!("{id}-{variant}"));
    if let Ok(s) = std::fs::read_to_string(&file) {
        return s;
    }
    let path = VARIANTS.iter().find(|(v, _)| *v == variant).map(|(_, p)| *p).expect("variant");
    let url = format!("https://www.youtube.com/s/player/{id}/{path}");
    let r = http.get(&url).send().await.unwrap_or_else(|e| panic!("fetching fixture player {id}/{variant} failed ({e}); run once online, or set ENCORE_EJS_PLAYERS to a folder that has them"));
    assert!(r.status().is_success(), "fixture player {id}/{variant}: {}", r.status());
    let body = r.text().await.unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let part = file.with_extension("part");
    std::fs::write(&part, &body).unwrap();
    std::fs::rename(&part, &file).unwrap();
    body
}

#[test]
fn the_corpus_is_the_pinned_list() {
    let pins = include_str!("../upstream.toml");
    for c in CASES {
        assert!(pins.contains(&format!("\"{}\"", c.player)), "{} is not in upstream.toml [fixtures]", c.player);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_fixture_player_solves() {
    let http = reqwest::Client::new();
    let solver = QuickJsSolver::new(Limits::default());
    let mut failures = Vec::new();
    let (mut cold_total, mut warm_total, mut cold_max, mut runs) = (0u128, 0u128, 0u128, 0u32);
    for case in CASES {
        for variant in case.variants {
            let code = player(&http, case.player, variant).await;
            let script = PlayerScript { id: format!("{}/{variant}", case.player), code: Arc::from(code) };
            let mut requests = Vec::new();
            if !case.n.is_empty() {
                requests.push(ChallengeRequest { kind: ChallengeType::N, challenges: case.n.iter().map(|(i, _)| i.to_string()).collect() });
            }
            if !case.sig.is_empty() {
                requests.push(ChallengeRequest { kind: ChallengeType::Sig, challenges: case.sig.iter().map(|(i, _)| i.to_string()).collect() });
            }
            for pass in ["cold", "preprocessed"] {
                let t = Instant::now();
                let out = solver.solve(&script, requests.clone()).await;
                let ms = t.elapsed().as_millis();
                if pass == "cold" {
                    cold_total += ms;
                    cold_max = cold_max.max(ms);
                } else {
                    warm_total += ms;
                }
                let out = match out {
                    Ok(o) => o,
                    Err(e) => {
                        failures.push(format!("{} {pass}: {e}", script.id));
                        continue;
                    }
                };
                for (req, res) in requests.iter().zip(out) {
                    let steps = if req.kind == ChallengeType::N { case.n } else { case.sig };
                    match res {
                        Ok(data) => {
                            for (input, expected) in steps {
                                if data.get(*input).map(String::as_str) != Some(*expected) {
                                    failures.push(format!("{} {pass} {:?} {:?}: got {:?}", script.id, req.kind, input, data.get(*input)));
                                }
                            }
                        }
                        Err(e) => failures.push(format!("{} {pass} {:?}: {e}", script.id, req.kind)),
                    }
                }
            }
            runs += 1;
        }
    }
    let n = runs.max(1) as u128;
    eprintln!("solved {runs} player variants: cold {} ms on average (slowest {cold_max} ms), preprocessed {} ms", cold_total / n, warm_total / n);
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
