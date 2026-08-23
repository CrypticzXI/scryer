#[cfg(test)]
mod episode_image_url_tests {
    use super::normalize_episode_image_url;

    #[test]
    fn accepts_fully_qualified_http_episode_image_urls() {
        assert_eq!(
            normalize_episode_image_url(" https://image.tmdb.org/t/p/original/still.jpg ")
                .as_deref(),
            Some("https://image.tmdb.org/t/p/original/still.jpg")
        );
        assert_eq!(
            normalize_episode_image_url("http://example.test/still.jpg").as_deref(),
            Some("http://example.test/still.jpg")
        );
    }

    #[test]
    fn rejects_non_fully_qualified_episode_image_urls() {
        for raw in [
            "",
            "/relative/still.jpg",
            "//image.tmdb.org/t/p/original/still.jpg",
            "data:image/png;base64,abc",
            "blob:https://example.test/id",
            "file:///tmp/still.jpg",
            "not-a-url",
        ] {
            assert_eq!(normalize_episode_image_url(raw), None, "raw={raw}");
        }
    }
}

#[cfg(test)]
mod title_credits_tests {
    use super::select_title_credits;
    use crate::TitleCredit;

    /// Cache rows arrive in position order; `billing_order` is what SMG ranked
    /// them by, so the two can disagree.
    fn credit(kind: &str, person: &str, billing_order: i32) -> TitleCredit {
        TitleCredit {
            kind: kind.to_string(),
            person_name: person.to_string(),
            billing_order,
            ..TitleCredit::default()
        }
    }

    fn names(credits: &[TitleCredit]) -> Vec<&str> {
        credits
            .iter()
            .map(|credit| credit.person_name.as_str())
            .collect()
    }

    #[test]
    fn orders_by_billing_order_then_cached_position() {
        let selected = select_title_credits(
            vec![
                credit("actor", "third", 2),
                credit("actor", "first", 0),
                credit("actor", "second-a", 1),
                credit("actor", "second-b", 1),
            ],
            None,
            10,
        );
        assert_eq!(
            names(&selected),
            vec!["first", "second-a", "second-b", "third"]
        );
    }

    #[test]
    fn filters_to_the_requested_kinds() {
        let credits = vec![
            credit("actor", "lead", 0),
            credit("director", "helmer", 1),
            credit("voice_actor", "dub", 2),
        ];
        let kinds = ["actor".to_string(), "voice_actor".to_string()];
        assert_eq!(
            names(&select_title_credits(credits.clone(), Some(&kinds), 10)),
            vec!["lead", "dub"]
        );
        // An absent or empty filter means "every kind", not "no kinds".
        assert_eq!(
            names(&select_title_credits(credits.clone(), None, 10)),
            vec!["lead", "helmer", "dub"]
        );
        assert_eq!(
            names(&select_title_credits(credits, Some(&[]), 10)),
            vec!["lead", "helmer", "dub"]
        );
    }

    #[test]
    fn truncates_after_ordering_so_the_top_billed_survive() {
        let selected = select_title_credits(
            vec![
                credit("actor", "eighth-billed", 7),
                credit("actor", "top-billed", 0),
            ],
            None,
            1,
        );
        assert_eq!(names(&selected), vec!["top-billed"]);
    }

    #[test]
    fn a_zero_limit_selects_nothing() {
        assert!(select_title_credits(vec![credit("actor", "lead", 0)], None, 0).is_empty());
    }
}
