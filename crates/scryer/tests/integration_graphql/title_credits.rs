//! GraphQL contract for `Title.credits`, the read side of the locally cached
//! SMG credit set.
//!
//! The rail on the title overview asks for cast only, so these pin the shape the
//! web depends on: kind filtering, billing-order ordering, the clamped limit,
//! proxied person portraits, and library-permission parity with every other
//! `Title` field.

use super::*;

use scryer_application::{TitleCredit, TitleMetadataUpdate};

const CREDIT_FIELDS: &str = r#"
    kind
    personName
    personOriginalName
    personImageUrl
    character
    language
    billingOrder
    episodeCount
"#;

fn credit(kind: &str, person: &str, billing_order: i32) -> TitleCredit {
    TitleCredit {
        kind: kind.to_string(),
        person_id: format!("person-{person}"),
        person_name: person.to_string(),
        person_source: "tmdb".to_string(),
        person_external_id: format!("tmdb-{person}"),
        billing_order,
        ..TitleCredit::default()
    }
}

async fn seed_title_credits(ctx: &TestContext, title_id: &str, credits: Vec<TitleCredit>) {
    TitleRepository::update_title_hydrated_metadata(
        &ctx.titles,
        title_id,
        TitleMetadataUpdate {
            metadata_fetched_at: Some(Utc::now().to_rfc3339()),
            credits: Some(credits),
            ..Default::default()
        },
    )
    .await
    .expect("seed cached title credits");
}

fn credit_names(credits: &Value) -> Vec<String> {
    credits
        .as_array()
        .expect("credits list")
        .iter()
        .map(|credit| {
            credit["personName"]
                .as_str()
                .expect("credit person name")
                .to_string()
        })
        .collect()
}

async fn query_credits(ctx: &TestContext, title_id: &str, arguments: &str) -> Value {
    let body = gql(
        ctx,
        &format!(
            r#"query($id: ID!) {{
                title(id: $id) {{
                    credits{arguments} {{{CREDIT_FIELDS}}}
                }}
            }}"#
        ),
        json!({ "id": title_id }),
    )
    .await;
    assert_no_errors(&body);
    body["data"]["title"]["credits"].clone()
}

#[tokio::test]
async fn graphql_title_credits_map_every_cached_field() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Credit Field Mapping",
        MediaFacet::Series,
        vec![],
        vec![],
        true,
    )
    .await;
    seed_title_credits(
        &ctx,
        &title.id,
        vec![TitleCredit {
            kind: "actor".to_string(),
            person_id: "person-1".to_string(),
            person_name: "Lead Actor".to_string(),
            person_original_name: "主演".to_string(),
            person_image_url: "https://image.tmdb.org/t/p/original/person-1.jpg".to_string(),
            person_source: "tmdb".to_string(),
            person_external_id: "tmdb-1".to_string(),
            character_name: "Hero".to_string(),
            language: "eng".to_string(),
            billing_order: 0,
            episode_count: Some(12),
        }],
    )
    .await;

    let credits = query_credits(&ctx, &title.id, "").await;
    let credit = &credits[0];
    assert_eq!(credit["kind"], "actor");
    assert_eq!(credit["personName"], "Lead Actor");
    assert_eq!(credit["personOriginalName"], "主演");
    assert_eq!(credit["character"], "Hero");
    assert_eq!(credit["language"], "eng");
    assert_eq!(credit["billingOrder"], 0);
    assert_eq!(credit["episodeCount"], 12);
    // Internal provenance (person id/source/external id) is cached but never
    // exposed: only the fields selected above exist on the payload.
    assert!(credit.get("personId").is_none());
    assert!(credit.get("personSource").is_none());
}

#[tokio::test]
async fn graphql_title_credit_portraits_use_the_opaque_media_image_route() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Credit Portrait Proxy",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;
    let mut with_image = credit("actor", "Pictured", 0);
    with_image.person_image_url = "https://image.tmdb.org/t/p/original/person-1.jpg".to_string();
    seed_title_credits(
        &ctx,
        &title.id,
        vec![with_image, credit("actor", "Unpictured", 1)],
    )
    .await;

    let credits = query_credits(&ctx, &title.id, "").await;
    let image_url = credits[0]["personImageUrl"]
        .as_str()
        .expect("cached portraits resolve to a proxied URL");
    let token = image_url
        .strip_prefix("/images/media/")
        .and_then(|value| value.strip_suffix("/w185"))
        .expect("credit portraits should use Scryer's media route at the person variant");
    assert_eq!(token.len(), 64);
    assert!(token.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!image_url.contains("image.tmdb.org"));
    assert!(
        credits[1]["personImageUrl"].is_null(),
        "a credit with no upstream portrait resolves to null, not a dead token"
    );

    let persisted: (Option<String>, String, String) = sqlx::query_as(
        "SELECT upstream_url, image_kind, fallback_class
           FROM image_proxy_sources
          WHERE token = ?",
    )
    .bind(token)
    .fetch_one(ctx.db.pool())
    .await
    .expect("the credit portrait source should be registered durably");
    assert_eq!(
        persisted,
        (
            Some("https://image.tmdb.org/t/p/original/person-1.jpg".to_string()),
            "person".to_string(),
            "portrait".to_string(),
        )
    );
}

#[tokio::test]
async fn graphql_title_credits_filter_by_kind_and_order_by_billing() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Credit Ordering",
        MediaFacet::Anime,
        vec![],
        vec![],
        true,
    )
    .await;
    // Cached in SMG response order, which deliberately disagrees with billing
    // order so the resolver's sort is observable.
    seed_title_credits(
        &ctx,
        &title.id,
        vec![
            credit("director", "Helmer", 0),
            credit("actor", "Third Billed", 3),
            credit("voice_actor", "Second Billed", 2),
            credit("actor", "Top Billed", 1),
        ],
    )
    .await;

    let cast = query_credits(&ctx, &title.id, r#"(kinds: ["actor", "voice_actor"])"#).await;
    assert_eq!(
        credit_names(&cast),
        vec!["Top Billed", "Second Billed", "Third Billed"],
        "cast filtering drops crew and orders by billing rank"
    );

    let all = query_credits(&ctx, &title.id, "").await;
    assert_eq!(
        credit_names(&all),
        vec!["Helmer", "Top Billed", "Second Billed", "Third Billed"],
        "an omitted kind filter returns every cached kind"
    );
}

#[tokio::test]
async fn graphql_title_credits_clamp_the_limit_and_default_to_an_empty_list() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Credit Limits",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;

    let empty = query_credits(&ctx, &title.id, "").await;
    assert_eq!(
        empty.as_array().map(Vec::len),
        Some(0),
        "a title with no cached credits returns an empty list, not null"
    );

    let credits = (0..60)
        .map(|index| credit("actor", &format!("Actor {index:02}"), index))
        .collect::<Vec<_>>();
    seed_title_credits(&ctx, &title.id, credits).await;

    let defaulted = query_credits(&ctx, &title.id, "").await;
    assert_eq!(
        defaulted.as_array().map(Vec::len),
        Some(15),
        "the resolver default keeps the rail at 15 credits"
    );
    let oversized = query_credits(&ctx, &title.id, "(limit: 500)").await;
    assert_eq!(
        oversized.as_array().map(Vec::len),
        Some(50),
        "an oversized limit clamps to 50"
    );
    let negative = query_credits(&ctx, &title.id, "(limit: -5)").await;
    assert_eq!(
        negative.as_array().map(Vec::len),
        Some(0),
        "a negative limit clamps to nothing rather than wrapping"
    );
    let pair = query_credits(&ctx, &title.id, "(limit: 2)").await;
    assert_eq!(
        credit_names(&pair),
        vec!["Actor 00", "Actor 01"],
        "the limit keeps the top-billed credits, not an arbitrary slice"
    );
}

#[tokio::test]
async fn graphql_title_credits_honor_library_view_permissions() {
    let ctx = TestContext::new().await;
    let title = create_catalog_title(
        &ctx,
        "Credit Authorization",
        MediaFacet::Movie,
        vec![],
        vec![],
        true,
    )
    .await;
    seed_title_credits(&ctx, &title.id, vec![credit("actor", "Lead", 0)]).await;

    let query = format!(
        r#"query {{
            title(id: "{}") {{ id ratings {{ rating }} credits {{ personName }} }}
        }}"#,
        title.id
    );
    let denied = schema_exec(&ctx, &query, Some(unauthorized_actor())).await;
    // Parity check: credits must fail exactly where the sibling `ratings` field
    // fails, i.e. the whole title resolves to null for an actor without view.
    assert!(
        denied["data"]["title"].is_null(),
        "an actor without library view permission sees no title, and so no credits: {denied}"
    );
}

fn unauthorized_actor() -> User {
    User {
        id: Id::new().0,
        username: "credits-outsider".to_string(),
        password_hash: None,
        password_change_required: false,
        account_kind: Default::default(),
        authorization: UserAuthorization {
            app: AppPermissionMask::NONE,
            libraries: HashMap::new(),
            default_library: LibraryPermissionMask::NONE,
            actor_capabilities: scryer_domain::ActorCapabilityMask::MANAGE_OWN_ACCOUNT,
            login_status: Default::default(),
            loaded: true,
        },
    }
}
