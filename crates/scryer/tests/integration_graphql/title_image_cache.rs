use super::*;

#[tokio::test]
async fn graphql_clear_title_image_cache_returns_opaque_success() {
    let ctx = TestContext::new().await;

    let mutation = r#"
        mutation ClearTitleImageCache {
          clearTitleImageCache {
            accepted
          }
        }
    "#;

    let first = gql(&ctx, mutation, json!({})).await;
    assert_no_errors(&first);
    assert_eq!(first["data"]["clearTitleImageCache"]["accepted"], true);

    let second = gql(&ctx, mutation, json!({})).await;
    assert_no_errors(&second);
    assert_eq!(second["data"]["clearTitleImageCache"]["accepted"], true);

    let unauthorized = schema_exec(&ctx, mutation, None).await;
    assert!(
        unauthorized["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "expected authorization error: {unauthorized}"
    );
}
