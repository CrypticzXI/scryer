use super::*;

#[tokio::test]
async fn graphql_clear_title_image_cache_returns_opaque_success() {
    let ctx = TestContext::new().await;

    let mutation = r#"
        mutation ClearTitleImageCache {
          clearTitleImageCache {
            requestedAt
          }
        }
    "#;

    let first = gql(&ctx, mutation, json!({})).await;
    assert_no_errors(&first);
    assert!(
        first["data"]["clearTitleImageCache"]["requestedAt"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "expected requestedAt timestamp: {first}"
    );

    let second = gql(&ctx, mutation, json!({})).await;
    assert_no_errors(&second);
    assert!(
        second["data"]["clearTitleImageCache"]["requestedAt"]
            .as_str()
            .is_some_and(|value| !value.is_empty()),
        "expected requestedAt timestamp: {second}"
    );

    let unauthorized = schema_exec(&ctx, mutation, None).await;
    assert!(
        unauthorized["errors"]
            .as_array()
            .is_some_and(|errors| !errors.is_empty()),
        "expected authorization error: {unauthorized}"
    );
}
