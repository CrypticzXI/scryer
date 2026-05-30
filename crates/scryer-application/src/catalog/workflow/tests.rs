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
