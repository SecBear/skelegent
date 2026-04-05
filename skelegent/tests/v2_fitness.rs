//! Architectural fitness guards for the v2 cutover.
//!
//! These tests scan the workspace at test time and fail if v2 invariants regress.
//! Run with: `cargo test -p skelegent v2_fitness`

use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Recursively collect all `*.rs` files under `root`, excluding paths that
/// contain any of the `excludes` substrings.
fn collect_rs_files(root: &Path, excludes: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_rs_files_inner(root, excludes, &mut out);
    out
}

fn collect_rs_files_inner(dir: &Path, excludes: &[&str], out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let path_str = path.to_string_lossy();

        // Apply exclusion filters
        if excludes.iter().any(|ex| path_str.contains(ex)) {
            continue;
        }

        if path.is_dir() {
            collect_rs_files_inner(&path, excludes, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Collect all `*.md` files directly under `dir` (non-recursive one level, or
/// recursive — here we only need the flat `specs/` level).
fn collect_md_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push(path);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Test 1 — v1 type names absent from workspace src
// ---------------------------------------------------------------------------
//
// These identifiers were deleted in the v2 cutover. Their original source
// files (effect.rs, effect_log.rs, effect_middleware.rs) are gone.
//
// NOTE: `EffectMiddleware` is intentionally excluded from this list because
// `skg-orch-kit` defines a *new* v2 `EffectMiddleware` trait for effect
// filtering that has the same name but different semantics and a different
// home (`orch/skg-orch-kit/src/runner.rs`). Checking for it would produce a
// false positive.  The v1 EffectMiddleware (from effect_middleware.rs) is gone.
//
// Matching strategy: skip pure-comment lines (trimmed starts with `//`) to
// avoid false positives from historical references in doc comments.  Any
// occurrence on a non-comment line is a genuine regression.

const BANNED_V1_IDENTIFIERS: &[&str] = &[
    "ExitReason",
    "OperatorError",
    "OrchError",
    "EffectEmitter",
    "EffectLog",
];

#[test]
fn test1_v1_type_names_absent() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    // skelegent/ lives one level below the workspace root
    let workspace_root = PathBuf::from(manifest_dir).join("..");

    let this_file = PathBuf::from(manifest_dir).join("tests/v2_fitness.rs");
    let this_file_canonical = this_file
        .canonicalize()
        .unwrap_or_else(|_| this_file.clone());

    let excludes = &[
        "/target/",
        "tests/golden/",
        "/.direnv/", // Nix-managed flake inputs — pinned snapshots of prior source
    ];

    // Walk `*/src/**/*.rs` — i.e. every crate's src tree under the workspace.
    // We collect from the workspace root and let the recursive walker honour
    // the exclusion list; the /target/ exclusion handles build artifacts.
    let files = collect_rs_files(&workspace_root, excludes);

    let mut violations: Vec<String> = Vec::new();

    for file_path in &files {
        // Skip the test file itself
        let canonical = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.clone());
        if canonical == this_file_canonical {
            continue;
        }

        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue, // skip unreadable files (e.g. non-UTF-8 fixtures)
        };

        for line in content.lines() {
            let trimmed = line.trim();
            // Skip pure comment lines; these may reference old names for
            // historical documentation without reintroducing the actual types.
            if trimmed.starts_with("//") {
                continue;
            }
            for &name in BANNED_V1_IDENTIFIERS {
                if trimmed.contains(name) {
                    violations.push(format!(
                        "  {} — contains banned v1 identifier `{}`",
                        file_path.display(),
                        name
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "v2 fitness: banned v1 identifiers found in non-comment source lines:\n{}",
        violations.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Test 2 — v1 specs carry RETIRED sentinel
// ---------------------------------------------------------------------------
//
// All v1 spec files that have been superseded by specs/v2/ must have a
// `> **RETIRED` sentinel on their first line.
//
// The following v1 spec files are legitimately NOT retired (they are either
// preserved as-is or deferred as follow-on work per specs/v2/00-overview
// migration matrix):
//   - 06-composition-factory-and-glue.md   (preserved; follow-on adoption)
//   - 09-hooks-lifecycle-and-governance.md (partially preserved; follow-on)
//   - 10-secrets-auth-crypto.md            (preserved; no v2 change in this pack)
//   - 11-testing-examples-and-backpressure.md  (mapped to v2/10, not yet retired)
//   - 12-packaging-versioning-and-umbrella-crate.md (partially mapped, not yet retired)
//   - 13-documentation-and-dx-parity.md    (preserved; documentation obligations apply to v2)
//
// Every other v1 spec file that is NOT in this set MUST carry the sentinel.

const ACTIVE_V1_SPECS: &[&str] = &[
    "06-composition-factory-and-glue.md",
    "09-hooks-lifecycle-and-governance.md",
    "10-secrets-auth-crypto.md",
    "11-testing-examples-and-backpressure.md",
    "12-packaging-versioning-and-umbrella-crate.md",
    "13-documentation-and-dx-parity.md",
];

#[test]
fn test2_v1_specs_carry_retired_sentinel() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let specs_dir = PathBuf::from(manifest_dir).join("../specs");

    let mut missing: Vec<String> = Vec::new();

    let files = collect_md_files(&specs_dir);
    for file_path in &files {
        let filename = file_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        // Skip active (not-yet-retired) v1 specs
        if ACTIVE_V1_SPECS.contains(&filename) {
            continue;
        }

        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(e) => {
                missing.push(format!(
                    "  {} — could not read file: {}",
                    file_path.display(),
                    e
                ));
                continue;
            }
        };

        let first_line = content.lines().next().unwrap_or("");
        if !first_line.starts_with("> **RETIRED") {
            missing.push(format!(
                "  {} — first line does not start with `> **RETIRED` (got: {:?})",
                file_path.display(),
                first_line
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "v2 fitness: v1 spec files missing the RETIRED sentinel:\n{}",
        missing.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Test 3 — v2 types accessible from the umbrella crate
// ---------------------------------------------------------------------------
//
// This is a compile-time check expressed as a runtime test.  If any of these
// types were removed from the public re-export path, the test binary would
// fail to compile rather than producing a test failure — which is the
// strongest possible signal.

#[test]
fn test3_v2_types_accessible_from_umbrella() {
    use std::any::TypeId;

    // These imports serve as the compile-time check.  If the umbrella crate
    // stops re-exporting any of these, this file will not compile.
    use skelegent::layer0::{CapabilityDescriptor, ExecutionEvent, Intent, Outcome, ProtocolError};

    // The runtime portion is minimal — just ensure the types are reachable
    // and their TypeIds can be materialised (proves they're concrete types,
    // not just re-exported trait aliases).
    let ids = [
        TypeId::of::<Intent>(),
        TypeId::of::<ExecutionEvent>(),
        TypeId::of::<Outcome>(),
        TypeId::of::<ProtocolError>(),
        TypeId::of::<CapabilityDescriptor>(),
    ];

    // All five TypeIds must be distinct (they're different types).
    let unique: std::collections::HashSet<TypeId> = ids.into_iter().collect();
    assert_eq!(
        unique.len(),
        5,
        "v2 fitness: expected 5 distinct TypeIds for the 5 umbrella-exported layer0 types"
    );
}
