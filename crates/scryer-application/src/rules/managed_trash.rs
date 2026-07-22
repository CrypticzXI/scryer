use scryer_domain::MediaFacet;
use scryer_release_parser::{TRASH_GUIDES_SOURCE_REVISION, TRASH_GUIDES_SYNCED_AT};

pub(crate) const MANAGED_TRASH_KEY_PREFIX: &str = "trash-guides:locale:";
const MANAGED_TRASH_REGISTRY_VERSION: &str = "managed-trash-registry-v1";

pub(crate) struct ManagedTrashRulePack {
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) applied_facets: &'static [MediaFacet],
    pub(crate) source: fn() -> String,
}

impl ManagedTrashRulePack {
    pub(crate) fn source(&self) -> String {
        (self.source)()
    }
}

pub(crate) fn managed_trash_rule_packs() -> &'static [ManagedTrashRulePack] {
    static PACKS: [ManagedTrashRulePack; 3] = [
        ManagedTrashRulePack {
            key: "trash-guides:locale:french",
            name: "TRaSH Guides French Locale",
            description: "Managed TRaSH Guides score-only locale pack for French audio intent.",
            applied_facets: &[],
            source: french_source,
        },
        ManagedTrashRulePack {
            key: "trash-guides:locale:german",
            name: "TRaSH Guides German Locale",
            description: "Managed TRaSH Guides score-only locale pack for German audio intent.",
            applied_facets: &[],
            source: german_source,
        },
        ManagedTrashRulePack {
            key: "trash-guides:locale:asian",
            name: "TRaSH Guides Asian Locale",
            description: "Managed TRaSH Guides score-only locale pack for the locale:asian tag.",
            applied_facets: &[],
            source: asian_source,
        },
    ];

    &PACKS
}

fn source(intent: &str, fact_prefix: &str, include_scene: bool, locale_rules: &str) -> String {
    let scene_rule = include_scene.then(|| {
        format!(
            r#"score_entry["trash_scene"] := -40 if {{
    locale_intent
    has_fact("{fact_prefix}.scene")
}}"#
        )
    });
    format!(
        r#"# MANAGED_TRASH_REGISTRY_VERSION={MANAGED_TRASH_REGISTRY_VERSION}
# TRASH_GUIDES_SYNCED_AT={TRASH_GUIDES_SYNCED_AT}
# TRASH_GUIDES_SOURCE_REVISION={TRASH_GUIDES_SOURCE_REVISION}
# This managed score-only policy is regenerated from the compiled locale-pack registry.

{intent}

has_fact(value) if {{
    some fact in input.release.guide_facts
    lower(fact) == value
}}

score_entry["trash_tier_1"] := 120 if {{
    locale_intent
    has_fact("{fact_prefix}.group.tier1")
}}

score_entry["trash_tier_2"] := 60 if {{
    locale_intent
    has_fact("{fact_prefix}.group.tier2")
}}

score_entry["trash_tier_3"] := 20 if {{
    locale_intent
    has_fact("{fact_prefix}.group.tier3")
}}

score_entry["trash_lq"] := -150 if {{
    locale_intent
    has_fact("{fact_prefix}.lq")
}}

{scene_rule}

{locale_rules}
"#,
        scene_rule = scene_rule.as_deref().unwrap_or_default(),
    )
}

fn french_source() -> String {
    source(
        r#"has_any_tag(values) if {
    some tag in input.context.tags
    some value in values
    lower(tag) == value
}

has_required_audio(values) if {
    some language in input.profile.required_audio_languages
    some value in values
    lower(language) == value
}

locale_intent if {
    has_required_audio(["fr", "fra", "fre", "french", "fr-fr", "fr-ca"])
}

locale_intent if {
    has_any_tag(["locale:fr", "locale:fr-fr", "locale:fr-ca"])
}

fr_fr_intent if {
    has_required_audio(["fr-fr"])
}

fr_fr_intent if {
    has_any_tag(["locale:fr-fr"])
}

fr_ca_intent if {
    has_required_audio(["fr-ca"])
}

fr_ca_intent if {
    has_any_tag(["locale:fr-ca"])
}

score_entry["trash_french_vostfr"] := -100 if {
    locale_intent
    has_fact("trash.locale.french.marker.vostfr")
}"#,
        "trash.locale.french",
        true,
        r#"regional_reference if {
    has_fact("trash.locale.french.marker.vff")
}

regional_reference if {
    has_fact("trash.locale.french.marker.vfi")
}

regional_reference if {
    has_fact("trash.locale.french.marker.vof")
}

regional_quebec if {
    has_fact("trash.locale.french.marker.vfq")
}

regional_quebec if {
    has_fact("trash.locale.french.marker.vq")
}

regional_quebec if {
    has_fact("trash.locale.french.marker.voq")
}

score_entry["trash_french_fr_fr_reference"] := 40 if {
    fr_fr_intent
    regional_reference
}

score_entry["trash_french_fr_fr_quebec"] := -20 if {
    fr_fr_intent
    regional_quebec
}

score_entry["trash_french_fr_ca_reference"] := -20 if {
    fr_ca_intent
    regional_reference
}

score_entry["trash_french_fr_ca_quebec"] := 40 if {
    fr_ca_intent
    regional_quebec
}"#,
    )
}

fn german_source() -> String {
    source(
        r#"has_any_tag(values) if {
    some tag in input.context.tags
    some value in values
    lower(tag) == value
}

has_required_audio(values) if {
    some language in input.profile.required_audio_languages
    some value in values
    lower(language) == value
}

locale_intent if {
    has_required_audio(["de", "deu", "ger", "german", "de-de"])
}

locale_intent if {
    has_any_tag(["locale:de", "locale:de-de"])
}"#,
        "trash.locale.german",
        true,
        r#"score_entry["trash_german_subbed"] := -100 if {
    locale_intent
    has_fact("trash.locale.german.marker.subbed")
}"#,
    )
}

fn asian_source() -> String {
    source(
        r#"locale_intent if {
    some tag in input.context.tags
    lower(tag) == "locale:asian"
}"#,
        "trash.locale.asian",
        false,
        "",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_stable_versioned_locale_keys() {
        let keys = managed_trash_rule_packs()
            .iter()
            .map(|pack| pack.key)
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "trash-guides:locale:french",
                "trash-guides:locale:german",
                "trash-guides:locale:asian",
            ]
        );
        assert!(
            managed_trash_rule_packs()[0]
                .source()
                .contains("MANAGED_TRASH_REGISTRY_VERSION=managed-trash-registry-v1")
        );
    }

    #[test]
    fn managed_packs_only_reference_locale_scene_facts_that_are_generated() {
        assert!(french_source().contains("trash.locale.french.scene"));
        assert!(german_source().contains("trash.locale.german.scene"));
        assert!(!asian_source().contains("trash.locale.asian.scene"));
    }
}
