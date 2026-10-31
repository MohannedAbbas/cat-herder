//! Harness version compatibility (`PLAN.md` §9): the control plane declares the
//! harness-image version range it supports and refuses to launch an image outside it,
//! so the Rust core and Python harness can't silently drift apart.

/// A parsed `major.minor.patch` semantic version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Semver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

/// Parse a `major.minor.patch` tag, tolerating a leading `v` and a `-prerelease` /
/// `+build` suffix. A missing `minor`/`patch` defaults to `0` (`1` → `1.0.0`,
/// `1.2` → `1.2.0`). Returns `None` for anything that isn't a plain version.
pub fn parse_semver(tag: &str) -> Option<Semver> {
    let t = tag.trim().trim_start_matches('v');
    // Strip prerelease (`-`) and build (`+`) suffixes; we only compare the core.
    let core = t.split('-').next()?.split('+').next()?;
    let mut it = core.split('.');
    let major = it.next()?.parse::<u64>().ok()?;
    let minor = it.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = it.next().unwrap_or("0").parse::<u64>().ok()?;
    // A fourth numeric component isn't `X.Y.Z`.
    if it.next().is_some() {
        return None;
    }
    Some(Semver {
        major,
        minor,
        patch,
    })
}

/// Extract the version tag from an image reference (`repo/image:tag`), or `None` when
/// the image has no tag (e.g. `image`, or `image@sha256:...`).
pub fn harness_version(image: &str) -> Option<Semver> {
    let without_digest = image.split('@').next().unwrap_or(image);
    let tag = without_digest.rsplit(':').next()?;
    if tag.is_empty() || tag == without_digest {
        // No `:tag` part (a bare image, or a digest-only pin).
        return None;
    }
    parse_semver(tag)
}

/// The harness version range the control plane supports, declared via
/// `CAT_HERDER_HARNESS_MIN_VERSION` / `CAT_HERDER_HARNESS_MAX_VERSION` (inclusive,
/// `None` = unbounded). The defaults below are the historical `0.1.x` harness.
#[derive(Debug, Clone, Copy, Default)]
pub struct VersionRange {
    pub min: Option<Semver>,
    pub max: Option<Semver>,
}

impl VersionRange {
    pub fn supports(&self, version: Semver) -> bool {
        self.min.map_or(true, |m| version >= m) && self.max.map_or(true, |m| version <= m)
    }

    pub fn from_env() -> Self {
        let parse = |key: &str| std::env::var(key).ok().and_then(|s| parse_semver(&s));
        Self {
            min: parse("CAT_HERDER_HARNESS_MIN_VERSION"),
            max: parse("CAT_HERDER_HARNESS_MAX_VERSION"),
        }
    }
}

/// Whether a harness image is known-incompatible with the control plane. A missing or
/// unparseable tag is treated as *compatible* — we can't verify a digest-pinned or
/// tagless image, so we don't refuse it; only a version we can parse and that falls
/// outside the declared range is refused.
pub fn is_incompatible(version: Option<Semver>, range: &VersionRange) -> bool {
    match version {
        Some(v) => !range.supports(v),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_semver_with_and_without_prefix_and_suffix() {
        assert_eq!(
            parse_semver("0.1.0"),
            Some(Semver {
                major: 0,
                minor: 1,
                patch: 0
            })
        );
        assert_eq!(parse_semver("v1.2.3"), parse_semver("1.2.3"));
        assert_eq!(
            parse_semver("1.2.3-rc1"),
            Some(Semver {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
        assert_eq!(
            parse_semver("1.2.3+build.5"),
            Some(Semver {
                major: 1,
                minor: 2,
                patch: 3
            })
        );
    }

    #[test]
    fn tolerates_missing_minor_and_patch() {
        assert_eq!(parse_semver("1"), parse_semver("1.0.0"));
        assert_eq!(parse_semver("1.2"), parse_semver("1.2.0"));
    }

    #[test]
    fn rejects_non_versions() {
        assert!(parse_semver("").is_none());
        assert!(parse_semver("latest").is_none());
        assert!(parse_semver("1.2.3.4").is_none());
        assert!(parse_semver("a.b.c").is_none());
    }

    #[test]
    fn extracts_tag_from_image_references() {
        assert_eq!(
            harness_version("ghcr.io/org/cat-herder-agent-harness:0.1.0"),
            Some(Semver {
                major: 0,
                minor: 1,
                patch: 0
            })
        );
        assert_eq!(
            harness_version("cat-herder-agent-harness:1.2.3@sha256:abc"),
            parse_semver("1.2.3")
        );
        assert!(harness_version("cat-herder-agent-harness").is_none());
        assert!(harness_version("cat-herder-agent-harness@sha256:abc").is_none());
    }

    #[test]
    fn range_supports_is_inclusive_and_optional() {
        let open = VersionRange::default();
        assert!(open.supports(parse_semver("0.0.1").unwrap()));

        let min = VersionRange {
            min: parse_semver("0.1.0"),
            max: None,
        };
        assert!(min.supports(parse_semver("0.1.0").unwrap()));
        assert!(min.supports(parse_semver("9.9.9").unwrap()));
        assert!(!min.supports(parse_semver("0.0.9").unwrap()));

        let bounded = VersionRange {
            min: parse_semver("0.1.0"),
            max: parse_semver("0.2.0"),
        };
        assert!(bounded.supports(parse_semver("0.1.5").unwrap()));
        assert!(!bounded.supports(parse_semver("0.3.0").unwrap()));
    }

    #[test]
    fn incompatibility_refuses_known_out_of_range_but_allows_unknown() {
        let range = VersionRange {
            min: parse_semver("0.1.0"),
            max: None,
        };
        // A known-too-old harness is refused.
        assert!(is_incompatible(parse_semver("0.0.9"), &range));
        assert!(!is_incompatible(parse_semver("0.1.0"), &range));
        // A tagless/digest image can't be verified — allow it.
        assert!(!is_incompatible(None, &range));
    }
}
