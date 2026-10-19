use super::*;
use crate::{
    catalogue::{DesignId, FolderId},
    encoding::query_pairs,
    fake::ScriptedTransport,
    transport::Method,
};

const NOW: u64 = 1_000;

fn onshape() -> Onshape {
    Onshape::new("client-123")
}

fn session() -> Session {
    Session {
        access_token: "access-1".to_owned(),
        refresh_token: Some("refresh-1".to_owned()),
        expires_at: NOW + 3600,
    }
}

fn redirect() -> Redirect {
    Redirect {
        uri: "http://localhost:49617/callback".to_owned(),
    }
}

fn form_of(request: &Request) -> Vec<(String, String)> {
    match &request.body {
        Body::Form(text) => query_pairs(text),
        other => panic!("expected a form, found {other:?}"),
    }
}

fn field(pairs: &[(String, String)], name: &str) -> Option<String> {
    pairs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
}

fn project() -> Project {
    Project {
        id: ProjectId::new("doc1"),
        workspace: WorkspaceId::new(Scope::Mine.id()),
        name: "Bracket".to_owned(),
    }
}

fn design(id: &str) -> Design {
    Design {
        id: DesignId::new(id),
        project: ProjectId::new("doc1"),
        name: "Studio".to_owned(),
        modified: None,
    }
}

fn first_page() -> String {
    format!("{API}/documents?filter=0&sortColumn=modifiedAt&sortOrder=desc&limit={DOCUMENT_PAGE}")
}

fn mine() -> WorkspaceId {
    WorkspaceId::new(Scope::Mine.id())
}

#[test]
fn the_sign_in_address_carries_pkce_the_redirect_and_a_state_but_no_secret() {
    let sign_in = onshape().start_sign_in(&redirect()).unwrap();

    let (base, query) = sign_in.url.split_once('?').unwrap();
    let pairs = query_pairs(query);

    assert_eq!(base, AUTHORIZE);
    assert_eq!(field(&pairs, "response_type").as_deref(), Some("code"));
    assert_eq!(field(&pairs, "client_id").as_deref(), Some("client-123"));
    assert_eq!(
        field(&pairs, "redirect_uri").as_deref(),
        Some("http://localhost:49617/callback")
    );
    assert_eq!(field(&pairs, "state"), Some(sign_in.state.clone()));
    assert_eq!(
        field(&pairs, "code_challenge"),
        Some(sign_in.verifier.challenge())
    );
    assert_eq!(
        field(&pairs, "code_challenge_method").as_deref(),
        Some("S256")
    );
    assert_eq!(field(&pairs, "client_secret"), None);
    assert!(!sign_in.url.contains(sign_in.verifier.as_str()));
}

#[test]
fn a_build_without_an_app_cannot_start_a_sign_in() {
    let result = Onshape::new(" ").start_sign_in(&redirect());

    assert!(matches!(
        result,
        Err(Error::NoClientId {
            service: Service::Onshape
        })
    ));
}

#[test]
fn finishing_the_sign_in_trades_the_code_and_verifier_without_a_secret() {
    let net = ScriptedTransport::new().route(
        Method::Post,
        TOKEN,
        200,
        r#"{"access_token":"a","refresh_token":"r","expires_in":3600,"token_type":"Bearer"}"#,
    );
    let provider = onshape();
    let sign_in = provider.start_sign_in(&redirect()).unwrap();
    let callback = Callback {
        code: "the-code".to_owned(),
        state: sign_in.state.clone(),
    };

    let session = provider
        .finish_sign_in(&net, &sign_in, &callback, NOW)
        .unwrap();

    let pairs = form_of(net.requests().first().unwrap());
    assert_eq!(
        field(&pairs, "grant_type").as_deref(),
        Some("authorization_code")
    );
    assert_eq!(field(&pairs, "code").as_deref(), Some("the-code"));
    assert_eq!(field(&pairs, "client_id").as_deref(), Some("client-123"));
    assert_eq!(
        field(&pairs, "code_verifier").as_deref(),
        Some(sign_in.verifier.as_str())
    );
    assert_eq!(field(&pairs, "client_secret"), None);
    assert_eq!(session.access_token, "a");
    assert_eq!(session.refresh_token.as_deref(), Some("r"));
    assert_eq!(session.expires_at, NOW + 3600);
}

#[test]
fn a_refresh_keeps_the_old_refresh_token_when_the_answer_has_none() {
    let net = ScriptedTransport::new().route(
        Method::Post,
        TOKEN,
        200,
        r#"{"access_token":"fresh","expires_in":600}"#,
    );

    let renewed = onshape().refresh(&net, &session(), NOW).unwrap();

    let pairs = form_of(net.requests().first().unwrap());
    assert_eq!(
        field(&pairs, "grant_type").as_deref(),
        Some("refresh_token")
    );
    assert_eq!(field(&pairs, "refresh_token").as_deref(), Some("refresh-1"));
    assert_eq!(field(&pairs, "client_secret"), None);
    assert_eq!(renewed.access_token, "fresh");
    assert_eq!(renewed.refresh_token.as_deref(), Some("refresh-1"));
}

#[test]
fn a_refused_refresh_means_signing_in_again() {
    let net =
        ScriptedTransport::new().route(Method::Post, TOKEN, 400, r#"{"error":"invalid_grant"}"#);

    assert!(matches!(
        onshape().refresh(&net, &session(), NOW),
        Err(Error::SignedOut)
    ));
}

#[test]
fn signing_out_sends_nothing_because_the_user_revokes_in_onshape() {
    let net = ScriptedTransport::new();

    onshape().revoke(&net, &session()).unwrap();

    assert!(net.requests().is_empty());
}

#[test]
fn the_account_is_the_session_info_name_then_the_email() {
    let url = format!("{API}/users/sessioninfo");
    let named = ScriptedTransport::new().route(
        Method::Get,
        &url,
        200,
        r#"{"name":"Ada Lovelace","email":"ada@example.com"}"#,
    );
    let unnamed = ScriptedTransport::new().route(
        Method::Get,
        &url,
        200,
        r#"{"name":" ","email":"ada@example.com"}"#,
    );
    let nothing = ScriptedTransport::new().route(Method::Get, &url, 200, "{}");

    assert_eq!(
        onshape().account(&named, &session()).unwrap().name,
        "Ada Lovelace"
    );
    assert_eq!(
        onshape().account(&unnamed, &session()).unwrap().name,
        "ada@example.com"
    );
    assert_eq!(
        onshape().account(&nothing, &session()).unwrap().name,
        "Onshape account"
    );
    assert_eq!(
        named.requests().first().unwrap().bearer.as_deref(),
        Some("access-1")
    );
}

#[test]
fn the_scopes_are_the_account_views_then_its_companies_and_teams() {
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &format!("{API}/companies"),
            200,
            r#"{"items":[{"id":"c1","name":"Acme"}]}"#,
        )
        .route(
            Method::Get,
            &format!("{API}/teams"),
            200,
            r#"{"items":[{"id":"t1","name":"Robotics","jsonType":"team"}],"next":null}"#,
        );

    let workspaces = onshape().workspaces(&net, &session()).unwrap();

    let listed: Vec<_> = workspaces
        .iter()
        .map(|workspace| (workspace.id.as_str(), workspace.name.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("documents", "My documents"),
            ("recent", "Recently opened"),
            ("shared", "Shared with me"),
            ("company/c1", "Acme (company)"),
            ("team/t1", "Robotics (team)"),
            ("public", "Public documents"),
        ]
    );
    assert_eq!(workspaces[0].kind, WorkspaceKind::Personal);
    assert!(
        net.requests()
            .iter()
            .all(|request| request.bearer.as_deref() == Some("access-1"))
    );
}

#[test]
fn an_account_without_company_or_team_access_still_lists_its_own_scopes() {
    let net = ScriptedTransport::new()
        .route(Method::Get, &format!("{API}/companies"), 403, "")
        .route(Method::Get, &format!("{API}/teams"), 500, "");

    let workspaces = onshape().workspaces(&net, &session()).unwrap();

    let ids: Vec<_> = workspaces
        .iter()
        .map(|workspace| workspace.id.as_str())
        .collect();
    assert_eq!(ids, ["documents", "recent", "shared", "public"]);
}

#[test]
fn a_lost_sign_in_while_listing_scopes_is_not_hidden() {
    let net = ScriptedTransport::new().route(Method::Get, &format!("{API}/companies"), 401, "");

    assert!(matches!(
        onshape().workspaces(&net, &session()),
        Err(Error::SignedOut)
    ));
}

#[test]
fn each_scope_asks_for_its_documents_with_the_documented_filter() {
    let cases = [
        (
            Scope::Shared,
            "filter=2&sortColumn=modifiedAt&sortOrder=desc",
        ),
        (
            Scope::Public,
            "filter=4&sortColumn=modifiedAt&sortOrder=desc",
        ),
        (Scope::Recent, "filter=5"),
        (
            Scope::Company("c 1".to_owned()),
            "filter=7&owner=c%201&ownerType=1&sortColumn=modifiedAt&sortOrder=desc",
        ),
        (
            Scope::Team("t1".to_owned()),
            "filter=9&owner=t1&sortColumn=modifiedAt&sortOrder=desc",
        ),
    ];
    for (scope, query) in cases {
        let url = format!("{API}/documents?{query}&limit={DOCUMENT_PAGE}");
        let net = ScriptedTransport::new().route(
            Method::Get,
            &url,
            200,
            r#"{"items":[{"id":"d1","name":"Bracket"}]}"#,
        );

        let projects = onshape()
            .projects(&net, &session(), &WorkspaceId::new(scope.id()))
            .unwrap();

        assert_eq!(projects.len(), 1, "{url}");
        assert_eq!(projects[0].workspace.as_str(), scope.id());
    }
}

#[test]
fn a_scope_that_is_not_ours_is_not_found() {
    let net = ScriptedTransport::new();

    for id in ["", "folder/f1", "team/", "company", "teams/t1"] {
        assert!(matches!(
            onshape().projects(&net, &session(), &WorkspaceId::new(id)),
            Err(Error::NotFound { .. })
        ));
    }
    assert!(net.requests().is_empty());
}

#[test]
fn only_documents_are_projects_when_a_listing_holds_folders_and_labels() {
    let net = ScriptedTransport::new().route(
        Method::Get,
        &first_page(),
        200,
        r#"{"items":[
            {"id":"f1","name":"Brackets","jsonType":"folder","resourceType":"folder","isContainer":true},
            {"id":"d1","name":"Bracket","jsonType":"document-summary","resourceType":"document"},
            {"id":"l1","name":"Draft","jsonType":"label"},
            {"id":"d2","name":"Gear","jsonType":"document"}
        ]}"#,
    );

    let projects = onshape().projects(&net, &session(), &mine()).unwrap();

    let ids: Vec<_> = projects.iter().map(|project| project.id.as_str()).collect();
    assert_eq!(ids, ["d1", "d2"]);
}

#[test]
fn documents_are_the_projects_and_paging_follows_next_inside_onshape() {
    let second = format!("{API}/documents?filter=0&offset=20&limit=20");
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &first_page(),
            200,
            &format!(
                r#"{{"items":[{{"id":"d1","name":"Bracket"}},{{"id":"d2","name":" "}}],"next":"{second}"}}"#
            ),
        )
        .route(
            Method::Get,
            &second,
            200,
            r#"{"items":[{"id":"d3","name":"Gear"}],"next":"https://elsewhere.example/documents"}"#,
        );

    let projects = onshape()
        .projects(&net, &session(), &WorkspaceId::new(Scope::Mine.id()))
        .unwrap();

    let names: Vec<_> = projects
        .iter()
        .map(|project| project.name.as_str())
        .collect();
    assert_eq!(names, ["Bracket", "Untitled", "Gear"]);
    assert_eq!(projects[0].id.as_str(), "d1");
    assert_eq!(net.requests().len(), 2);
}

#[test]
fn a_document_lists_its_part_studios_and_assemblies_from_its_default_workspace() {
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &format!("{API}/documents/doc1"),
            200,
            r#"{"id":"doc1","defaultWorkspace":{"id":"ws9"}}"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/w/ws9/elements"),
            200,
            r#"[
                {"id":"e1","name":"Frame","elementType":"PARTSTUDIO"},
                {"id":"e2","name":"Drawing 1","elementType":"DRAWING"},
                {"id":"e3","name":"Whole bike","elementType":"ASSEMBLY"},
                {"id":"e4","name":"scan.step","elementType":"BLOB"}
            ]"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/workspaces"),
            200,
            r#"[{"id":"ws9","name":"Main"}]"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/versions?limit={VERSION_LIMIT}"),
            200,
            "[]",
        );

    let root = onshape().root(&net, &session(), &project()).unwrap();

    assert!(root.folders.is_empty());
    let designs: Vec<_> = root
        .designs
        .iter()
        .map(|design| (design.name.as_str(), design.id.as_str()))
        .collect();
    assert_eq!(
        designs,
        [
            ("Frame", "partstudios/w/ws9/e1"),
            ("Whole bike", "assemblies/w/ws9/e3"),
        ]
    );
    assert!(
        root.designs
            .iter()
            .all(|design| design.project.as_str() == "doc1")
    );
}

#[test]
fn a_document_offers_its_other_branches_and_its_versions_newest_first() {
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &format!("{API}/documents/doc1"),
            200,
            r#"{"id":"doc1","defaultWorkspace":{"id":"ws9"}}"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/w/ws9/elements"),
            200,
            r#"[{"id":"e1","name":"Frame","elementType":"PARTSTUDIO"}]"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/workspaces"),
            200,
            r#"[
                {"id":"ws9","name":"Main","documentId":"doc1","type":"workspace"},
                {"id":"ws3","name":"Lighter frame","documentId":"doc1"},
                {"id":"ws2","name":"Alloy","documentId":"doc1"}
            ]"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/versions?limit={VERSION_LIMIT}"),
            200,
            r#"[
                {"id":"v1","name":"Start","createdAt":"2024-01-02T10:00:00.000Z"},
                {"id":"v3","name":"Release 2","createdAt":"2024-06-01T08:30:00.000Z"},
                {"id":"v2","name":"Release 1","createdAt":"2024-03-15T12:00:00.000Z"}
            ]"#,
        );

    let root = onshape().root(&net, &session(), &project()).unwrap();

    let folders: Vec<_> = root
        .folders
        .iter()
        .map(|entry| (entry.name.as_str(), entry.reference.folder.as_str()))
        .collect();
    assert_eq!(
        folders,
        [
            ("Branch Alloy", "w/ws2"),
            ("Branch Lighter frame", "w/ws3"),
            ("Version Release 2", "v/v3"),
            ("Version Release 1", "v/v2"),
            ("Version Start", "v/v1"),
        ]
    );
    assert!(
        root.folders
            .iter()
            .all(|entry| entry.reference.project.as_str() == "doc1")
    );
    assert_eq!(root.designs.len(), 1);
}

#[test]
fn a_branch_and_a_version_list_their_own_elements_and_import_from_them() {
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/w/ws3/elements"),
            200,
            r#"[{"id":"e1","name":"Frame","elementType":"PARTSTUDIO"}]"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/v/v3/elements"),
            200,
            r#"[
                {"id":"e7","name":"Rig","elementType":"ASSEMBLY"},
                {"id":"e2","name":"Frame","elementType":"PARTSTUDIO"},
                {"id":"e8","name":"Sheet","elementType":"DRAWING"}
            ]"#,
        )
        .route(
            Method::Post,
            &format!("{API}/assemblies/d/doc1/v/v3/e/e7/translations"),
            200,
            r#"{"id":"tr9"}"#,
        );
    let reference = |folder: &str| FolderRef {
        project: ProjectId::new("doc1"),
        folder: FolderId::new(folder),
    };

    let branch = onshape()
        .folder(&net, &session(), &reference("w/ws3"))
        .unwrap();
    let version = onshape()
        .folder(&net, &session(), &reference("v/v3"))
        .unwrap();

    assert!(branch.folders.is_empty() && version.folders.is_empty());
    assert_eq!(branch.designs[0].id.as_str(), "partstudios/w/ws3/e1");
    let ids: Vec<_> = version
        .designs
        .iter()
        .map(|design| design.id.as_str())
        .collect();
    assert_eq!(ids, ["partstudios/v/v3/e2", "assemblies/v/v3/e7"]);

    let translation = onshape()
        .start_conversion(&net, &session(), &version.designs[1], None)
        .unwrap();
    assert_eq!(translation.source, "doc1/tr9");
}

#[test]
fn a_folder_that_is_not_ours_is_not_found() {
    let net = ScriptedTransport::new();

    for folder in ["", "ws3", "x/ws3", "w/", "w/a/b"] {
        let reference = FolderRef {
            project: ProjectId::new("doc1"),
            folder: FolderId::new(folder),
        };
        assert!(matches!(
            onshape().folder(&net, &session(), &reference),
            Err(Error::NotFound {
                subject: Subject::Folder
            })
        ));
    }
    assert!(net.requests().is_empty());
}

#[test]
fn a_document_still_lists_its_designs_when_branches_and_versions_are_refused() {
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &format!("{API}/documents/doc1"),
            200,
            r#"{"id":"doc1","defaultWorkspace":{"id":"ws9"}}"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/w/ws9/elements"),
            200,
            r#"[{"id":"e1","name":"Frame","elementType":"PARTSTUDIO"}]"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/workspaces"),
            403,
            "",
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/versions?limit={VERSION_LIMIT}"),
            200,
            "not json",
        );

    let root = onshape().root(&net, &session(), &project()).unwrap();

    assert!(root.folders.is_empty());
    assert_eq!(root.designs.len(), 1);
}

#[test]
fn a_document_that_is_gone_says_not_found() {
    let net =
        ScriptedTransport::new().route(Method::Get, &format!("{API}/documents/doc1"), 404, "");

    assert!(matches!(
        onshape().root(&net, &session(), &project()),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn starting_a_conversion_asks_for_step_kept_outside_the_document() {
    let url = format!("{API}/partstudios/d/doc1/w/ws9/e/e1/translations");
    let net = ScriptedTransport::new().route(
        Method::Post,
        &url,
        200,
        r#"{"id":"tr1","requestState":"ACTIVE"}"#,
    );

    let translation = onshape()
        .start_conversion(&net, &session(), &design("partstudios/w/ws9/e1"), None)
        .unwrap();

    assert_eq!(translation.source, "doc1/tr1");
    let request = net.requests().into_iter().next().unwrap();
    assert_eq!(request.bearer.as_deref(), Some("access-1"));
    let Body::Json(body) = request.body else {
        panic!("expected a JSON body");
    };
    let sent: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(sent["formatName"], "STEP");
    assert_eq!(sent["storeInDocument"], false);
}

#[test]
fn an_assembly_converts_through_its_own_endpoint() {
    let url = format!("{API}/assemblies/d/doc1/w/ws9/e/e3/translations");
    let net = ScriptedTransport::new().route(Method::Post, &url, 200, r#"{"id":"tr2"}"#);

    let translation = onshape()
        .start_conversion(&net, &session(), &design("assemblies/w/ws9/e3"), None)
        .unwrap();

    assert_eq!(translation.source, "doc1/tr2");
}

#[test]
fn a_design_id_that_is_not_ours_is_not_found() {
    let net = ScriptedTransport::new();

    for id in [
        "",
        "e1",
        "partstudios/w/ws9",
        "partstudios/ws9/e1",
        "folders/w/ws9/e1",
        "partstudios/x/ws9/e1",
        "partstudios/w/ws9/e1/x",
    ] {
        assert!(matches!(
            onshape().start_conversion(&net, &session(), &design(id), None),
            Err(Error::NotFound {
                subject: Subject::Design
            })
        ));
    }
    assert!(net.requests().is_empty());
}

#[test]
fn polling_reports_working_then_the_external_file_when_done() {
    let url = format!("{API}/translations/tr1");
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &url,
            200,
            r#"{"id":"tr1","requestState":"ACTIVE"}"#,
        )
        .route(
            Method::Get,
            &url,
            200,
            r#"{"id":"tr1","requestState":"DONE","resultExternalDataIds":["file7"]}"#,
        );
    let translation = Translation {
        source: "doc1/tr1".to_owned(),
    };

    let first = onshape()
        .poll_conversion(&net, &session(), &translation)
        .unwrap();
    let second = onshape()
        .poll_conversion(&net, &session(), &translation)
        .unwrap();

    assert_eq!(first, Conversion::Working { percent: None });
    assert_eq!(
        second,
        Conversion::Ready(Download {
            source: "doc1".to_owned(),
            artifact: "file7".to_owned(),
        })
    );
}

#[test]
fn a_failed_translation_and_one_without_a_file_are_told_apart() {
    let translation = Translation {
        source: "doc1/tr1".to_owned(),
    };
    let url = format!("{API}/translations/tr1");
    let failed = ScriptedTransport::new().route(
        Method::Get,
        &url,
        200,
        r#"{"requestState":"FAILED","failureReason":"x"}"#,
    );
    let empty = ScriptedTransport::new().route(
        Method::Get,
        &url,
        200,
        r#"{"requestState":"DONE","resultExternalDataIds":[]}"#,
    );

    assert!(matches!(
        onshape().poll_conversion(&failed, &session(), &translation),
        Err(Error::ConversionFailed { .. })
    ));
    assert!(matches!(
        onshape().poll_conversion(&empty, &session(), &translation),
        Err(Error::NoStepFile { .. })
    ));
}

#[test]
fn a_translation_source_without_a_document_is_not_found() {
    let net = ScriptedTransport::new();
    let translation = Translation {
        source: "tr1".to_owned(),
    };

    assert!(matches!(
        onshape().poll_conversion(&net, &session(), &translation),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn fetching_streams_the_external_file_with_the_bearer() {
    let url = format!("{API}/documents/d/doc1/externaldata/file7");
    let net = ScriptedTransport::new().route(Method::Get, &url, 200, "ISO-10303-21;");
    let download = Download {
        source: "doc1".to_owned(),
        artifact: "file7".to_owned(),
    };
    let mut sink = Vec::new();

    let bytes = onshape()
        .fetch(&net, &session(), &download, &mut sink, &mut |_, _| true)
        .unwrap();

    assert_eq!(sink, b"ISO-10303-21;");
    assert_eq!(bytes, 13);
    assert_eq!(
        net.requests().first().unwrap().bearer.as_deref(),
        Some("access-1")
    );
}

#[test]
fn a_refused_download_is_an_error_not_a_file() {
    let url = format!("{API}/documents/d/doc1/externaldata/file7");
    let net = ScriptedTransport::new().route(Method::Get, &url, 403, "no");
    let download = Download {
        source: "doc1".to_owned(),
        artifact: "file7".to_owned(),
    };

    let result = onshape().fetch(&net, &session(), &download, &mut Vec::new(), &mut |_, _| {
        true
    });

    assert!(matches!(result, Err(Error::Forbidden { .. })));
}

#[test]
fn the_status_codes_of_a_listing_map_to_typed_errors() {
    let url = first_page();
    for (status, expected) in [(401, "signed out"), (429, "rate limited"), (500, "refused")] {
        let net = ScriptedTransport::new().route(Method::Get, &url, status, "");
        let error = onshape()
            .projects(&net, &session(), &WorkspaceId::new(Scope::Mine.id()))
            .unwrap_err();
        let kind = match error {
            Error::SignedOut => "signed out",
            Error::RateLimited { .. } => "rate limited",
            Error::Refused { .. } => "refused",
            _ => "other",
        };
        assert_eq!(kind, expected);
    }
}

fn workspace_place() -> FolderRef {
    FolderRef {
        project: ProjectId::new("doc1"),
        folder: Place::workspace("ws9").folder_id(),
    }
}

fn elements_url() -> String {
    format!("{API}/documents/d/doc1/w/ws9/elements")
}

fn blob_url() -> String {
    format!("{API}/blobelements/d/doc1/w/ws9")
}

fn send_to(net: &ScriptedTransport, name: &str) -> Result<Sent, Error> {
    let place = workspace_place();
    onshape().send(
        net,
        &session(),
        &Outgoing {
            folder: &place,
            name,
            bytes: b"ISO-10303-21;",
        },
        &mut |_, _| true,
    )
}

fn upload_of(request: &Request) -> &Multipart {
    match &request.body {
        Body::Multipart(multipart) => multipart,
        other => panic!("expected a multipart upload, found {other:?}"),
    }
}

#[test]
fn a_model_is_uploaded_as_a_new_blob_element_of_the_workspace() {
    let net = ScriptedTransport::new()
        .route(Method::Get, &elements_url(), 200, "[]")
        .route(
            Method::Post,
            &blob_url(),
            200,
            r#"{"id":"e7","name":"plate.step"}"#,
        );
    let mut seen = Vec::new();

    let place = workspace_place();
    let sent = onshape()
        .send(
            &net,
            &session(),
            &Outgoing {
                folder: &place,
                name: " plate.step ",
                bytes: b"ISO-10303-21;",
            },
            &mut |done, total| {
                seen.push((done, total));
                true
            },
        )
        .unwrap();

    assert_eq!(sent.name, "plate.step");
    assert_eq!(sent.outcome, SendOutcome::Created);
    assert_eq!(seen.first(), Some(&(0, Some(13))));
    assert_eq!(seen.last(), Some(&(13, Some(13))));
    assert!(seen.iter().all(|(_, total)| *total == Some(13)));
    assert!(seen.is_sorted());
    let requests = net.requests();
    let post = requests.last().unwrap();
    assert_eq!(post.method, Method::Post);
    assert_eq!(post.bearer.as_deref(), Some("access-1"));
    let upload = upload_of(post);
    assert_eq!(upload.file_name(), "plate.step");
    assert_eq!(upload.file_bytes(), b"ISO-10303-21;");
    assert_eq!(
        field(upload.fields(), "translate").as_deref(),
        Some("false")
    );
    assert_eq!(
        field(upload.fields(), "storeInDocument").as_deref(),
        Some("true")
    );
    assert_eq!(
        field(upload.fields(), "encodedFilename").as_deref(),
        Some("plate.step")
    );
}

#[test]
fn a_blob_of_the_same_name_is_updated_and_a_design_of_it_is_not_taken_for_one() {
    let listing = r#"[
        {"id":"e1","name":"PLATE.step","elementType":"PARTSTUDIO"},
        {"id":"e4","name":"Plate.STEP","elementType":"BLOB"}
    ]"#;
    let net = ScriptedTransport::new()
        .route(Method::Get, &elements_url(), 200, listing)
        .route(
            Method::Post,
            &format!("{}/e/e4", blob_url()),
            200,
            r#"{"id":"e4"}"#,
        );

    let sent = send_to(&net, "plate.step").unwrap();

    assert_eq!(sent.outcome, SendOutcome::NewVersion { number: None });
    assert_eq!(net.requests().len(), 2);

    let only_a_design = ScriptedTransport::new()
        .route(
            Method::Get,
            &elements_url(),
            200,
            r#"[{"id":"e1","name":"plate.step","elementType":"PARTSTUDIO"}]"#,
        )
        .route(Method::Post, &blob_url(), 200, "{}");

    assert_eq!(
        send_to(&only_a_design, "plate.step").unwrap().outcome,
        SendOutcome::Created
    );
}

#[test]
fn a_name_with_quotes_or_accents_cannot_break_out_of_the_upload_headers() {
    let net = ScriptedTransport::new()
        .route(Method::Get, &elements_url(), 200, "[]")
        .route(Method::Post, &blob_url(), 200, "{}");

    send_to(&net, "pla\"te é.step").unwrap();

    let requests = net.requests();
    let upload = upload_of(requests.last().unwrap());
    assert_eq!(upload.file_name(), "pla\"te é.step");
    assert_eq!(
        field(upload.fields(), "encodedFilename").as_deref(),
        Some("pla%22te%20%C3%A9.step")
    );
    let body = String::from_utf8_lossy(&upload.encoded()).into_owned();
    assert!(body.contains("filename=\"pla%22te%20%C3%A9.step\""));
    assert!(!body.contains("filename=\"pla\"te"));
}

#[test]
fn nothing_is_sent_into_a_saved_version_or_a_folder_that_is_not_ours() {
    let net = ScriptedTransport::new();
    let version = FolderRef {
        project: ProjectId::new("doc1"),
        folder: Place::version("v1").folder_id(),
    };
    let stray = FolderRef {
        project: ProjectId::new("doc1"),
        folder: crate::catalogue::FolderId::new("top"),
    };

    for (place, expect_read_only) in [(&version, true), (&stray, false)] {
        let result = onshape().send(
            &net,
            &session(),
            &Outgoing {
                folder: place,
                name: "plate.step",
                bytes: b"x",
            },
            &mut |_, _| true,
        );

        assert_eq!(
            matches!(result, Err(Error::VersionIsReadOnly)),
            expect_read_only
        );
        assert!(result.is_err());
    }
    assert!(net.requests().is_empty());
}

#[test]
fn the_status_codes_of_an_upload_map_to_typed_errors() {
    for (status, expected) in [
        (401, "signed out"),
        (403, "forbidden"),
        (429, "rate limited"),
        (400, "refused"),
    ] {
        let net = ScriptedTransport::new()
            .route(Method::Get, &elements_url(), 200, "[]")
            .route(Method::Post, &blob_url(), status, "");

        let kind = match send_to(&net, "plate.step").unwrap_err() {
            Error::SignedOut => "signed out",
            Error::SendForbidden { .. } => "forbidden",
            Error::RateLimited { .. } => "rate limited",
            Error::Refused { .. } => "refused",
            _ => "other",
        };

        assert_eq!(kind, expected);
    }

    let garbled = ScriptedTransport::new()
        .route(Method::Get, &elements_url(), 200, "[]")
        .route(Method::Post, &blob_url(), 200, "not json");

    assert!(matches!(
        send_to(&garbled, "plate.step"),
        Err(Error::Unreadable {
            subject: Subject::Item,
            ..
        })
    ));
}

#[test]
fn stopping_before_the_upload_sends_no_file() {
    let net = ScriptedTransport::new()
        .route(Method::Get, &elements_url(), 200, "[]")
        .route(Method::Post, &blob_url(), 200, "{}");
    let place = workspace_place();

    let result = onshape().send(
        &net,
        &session(),
        &Outgoing {
            folder: &place,
            name: "plate.step",
            bytes: b"x",
        },
        &mut |_, _| false,
    );

    assert!(matches!(result, Err(Error::Stopped)));
    assert!(
        net.requests()
            .iter()
            .all(|request| request.method == Method::Get)
    );
}

#[test]
fn uploaded_files_are_listed_with_their_place_and_download_as_blobs() {
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &format!("{API}/documents/doc1"),
            200,
            r#"{"id":"doc1","defaultWorkspace":{"id":"ws9"}}"#,
        )
        .route(
            Method::Get,
            &elements_url(),
            200,
            r#"[
                {"id":"e1","name":"Frame","elementType":"PARTSTUDIO"},
                {"id":"e4","name":"scan.STEP","elementType":"BLOB"},
                {"id":"e5","name":"notes.pdf","elementType":"BLOB"},
                {"id":"e6","name":"outline.svg","elementType":"BLOB"}
            ]"#,
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/workspaces"),
            200,
            "[]",
        )
        .route(
            Method::Get,
            &format!("{API}/documents/d/doc1/versions?limit={VERSION_LIMIT}"),
            200,
            "[]",
        )
        .route_bytes(
            Method::Get,
            &format!("{API}/blobelements/d/doc1/w/ws9/e/e4"),
            200,
            b"ISO-10303-21;",
        );

    let root = onshape().root(&net, &session(), &project()).unwrap();

    assert_eq!(root.place, Some(workspace_place()));
    let uploads: Vec<_> = root
        .uploads
        .iter()
        .map(|upload| (upload.name.as_str(), upload.format, upload.id.as_str()))
        .collect();
    assert_eq!(
        uploads,
        [
            ("outline.svg", UploadFormat::Svg, "w/ws9/e6"),
            ("scan.STEP", UploadFormat::Step, "w/ws9/e4"),
        ]
    );

    let step = root
        .uploads
        .iter()
        .find(|upload| upload.format == UploadFormat::Step)
        .unwrap();
    let mut sink = Vec::new();
    let bytes = onshape()
        .fetch_upload(&net, &session(), step, &mut sink, &mut |_, _| true)
        .unwrap();

    assert_eq!(bytes, 13);
    assert_eq!(sink, b"ISO-10303-21;");
}

#[test]
fn a_branch_accepts_files_and_a_version_does_not() {
    let listing = |url: &str| {
        ScriptedTransport::new().route(
            Method::Get,
            url,
            200,
            r#"[{"id":"e1","name":"Frame","elementType":"PARTSTUDIO"}]"#,
        )
    };
    let branch = FolderRef {
        project: ProjectId::new("doc1"),
        folder: Place::workspace("ws2").folder_id(),
    };
    let version = FolderRef {
        project: ProjectId::new("doc1"),
        folder: Place::version("v1").folder_id(),
    };

    let in_branch = onshape()
        .folder(
            &listing(&format!("{API}/documents/d/doc1/w/ws2/elements")),
            &session(),
            &branch,
        )
        .unwrap();
    let in_version = onshape()
        .folder(
            &listing(&format!("{API}/documents/d/doc1/v/v1/elements")),
            &session(),
            &version,
        )
        .unwrap();

    assert_eq!(in_branch.place, Some(branch));
    assert_eq!(in_version.place, None);
}

#[test]
fn an_upload_that_is_not_ours_is_not_found() {
    let net = ScriptedTransport::new();
    let upload = Upload {
        id: crate::catalogue::UploadId::new("nonsense"),
        project: ProjectId::new("doc1"),
        name: "a.step".to_owned(),
        format: UploadFormat::Step,
        modified: None,
    };

    let result =
        onshape().fetch_upload(&net, &session(), &upload, &mut Vec::new(), &mut |_, _| true);

    assert!(matches!(
        result,
        Err(Error::NotFound {
            subject: Subject::Upload
        })
    ));
}

fn send_part_studio_to(
    net: &ScriptedTransport,
    place: &FolderRef,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<Translation, Error> {
    onshape().send_part_studio(
        net,
        &session(),
        &Outgoing {
            folder: place,
            name: " plate.step ",
            bytes: b"ISO-10303-21;",
        },
        progress,
    )
}

const STARTED: &str = r#"{
    "id":"tr5","requestState":"ACTIVE","documentId":"doc1","workspaceId":"ws9",
    "resultElementIds":null,"resultExternalDataIds":null,"resultDocumentId":null,
    "failureReason":null,"name":"plate.step"
}"#;

#[test]
fn a_started_translation_with_null_result_lists_is_read() {
    let net = ScriptedTransport::new().route(
        Method::Post,
        &format!("{API}/partstudios/d/doc1/w/ws9/e/e1/translations"),
        200,
        STARTED,
    );

    let translation = onshape()
        .start_conversion(&net, &session(), &design("partstudios/w/ws9/e1"), None)
        .unwrap();

    assert_eq!(translation.source, "doc1/tr5");
}

#[test]
fn a_model_sent_as_a_part_studio_is_uploaded_for_translation_and_the_translation_returned() {
    let net = ScriptedTransport::new().route(Method::Post, &blob_url(), 200, STARTED);
    let mut seen = Vec::new();

    let translation = send_part_studio_to(&net, &workspace_place(), &mut |done, total| {
        seen.push((done, total));
        true
    })
    .unwrap();

    assert_eq!(translation.source, "doc1/tr5");
    assert_eq!(seen.first(), Some(&(0, Some(13))));
    assert_eq!(seen.last(), Some(&(13, Some(13))));
    let requests = net.requests();
    assert!(
        requests
            .iter()
            .all(|request| request.method == Method::Post)
    );
    let upload = upload_of(requests.last().unwrap());
    assert_eq!(upload.file_name(), "plate.step");
    assert_eq!(upload.file_bytes(), b"ISO-10303-21;");
    for (name, value) in [
        ("translate", "true"),
        ("formatName", "STEP"),
        ("storeInDocument", "false"),
        ("encodedFilename", "plate.step"),
    ] {
        assert_eq!(field(upload.fields(), name).as_deref(), Some(value));
    }
}

#[test]
fn stopping_while_the_body_goes_out_ends_the_send_without_a_translation() {
    let net = ScriptedTransport::new().route(Method::Post, &blob_url(), 200, STARTED);
    let mut calls = 0;

    let result = send_part_studio_to(&net, &workspace_place(), &mut |_, _| {
        calls += 1;
        calls < 2
    });

    assert!(result.unwrap_err().is_stop());
}

#[test]
fn a_blob_stopped_while_the_body_goes_out_is_stopped_too() {
    let net = ScriptedTransport::new()
        .route(Method::Get, &elements_url(), 200, "[]")
        .route(Method::Post, &blob_url(), 200, "{}");
    let place = workspace_place();
    let mut calls = 0;

    let result = onshape().send(
        &net,
        &session(),
        &Outgoing {
            folder: &place,
            name: "plate.step",
            bytes: b"x",
        },
        &mut |_, _| {
            calls += 1;
            calls < 2
        },
    );

    assert!(result.unwrap_err().is_stop());
}

#[test]
fn nothing_is_translated_into_a_saved_version_and_the_upload_answers_map_to_typed_errors() {
    let version = FolderRef {
        project: ProjectId::new("doc1"),
        folder: Place::version("v1").folder_id(),
    };
    let net = ScriptedTransport::new();

    assert!(matches!(
        send_part_studio_to(&net, &version, &mut |_, _| true),
        Err(Error::VersionIsReadOnly)
    ));
    assert!(net.requests().is_empty());

    for (status, body, expected) in [
        (403, "", "forbidden"),
        (401, "", "signed out"),
        (200, "not json", "unreadable"),
        (200, r#"{"requestState":"ACTIVE"}"#, "unreadable"),
    ] {
        let net = ScriptedTransport::new().route(Method::Post, &blob_url(), status, body);

        let kind =
            match send_part_studio_to(&net, &workspace_place(), &mut |_, _| true).unwrap_err() {
                Error::SendForbidden { .. } => "forbidden",
                Error::SignedOut => "signed out",
                Error::Unreadable { .. } => "unreadable",
                _ => "other",
            };

        assert_eq!(kind, expected);
    }
}

#[test]
fn a_part_studio_translation_is_polled_until_it_made_an_element() {
    let url = format!("{API}/translations/tr5");
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &url,
            200,
            r#"{"id":"tr5","requestState":"ACTIVE","resultElementIds":null}"#,
        )
        .route(
            Method::Get,
            &url,
            200,
            r#"{"id":"tr5","requestState":"DONE","resultElementIds":["e12"]}"#,
        );
    let translation = Translation {
        source: "doc1/tr5".to_owned(),
    };

    let first = onshape().poll_part_studio(&net, &session(), &translation);
    let second = onshape().poll_part_studio(&net, &session(), &translation);

    assert_eq!(first.unwrap(), Placement::Working);
    assert_eq!(second.unwrap(), Placement::Done);
}

#[test]
fn a_failed_translation_or_one_that_made_nothing_is_a_failure_to_make_a_part_studio() {
    let translation = Translation {
        source: "doc1/tr5".to_owned(),
    };
    let url = format!("{API}/translations/tr5");

    for body in [
        r#"{"requestState":"FAILED","failureReason":"x","resultElementIds":null}"#,
        r#"{"requestState":"DONE","resultElementIds":[]}"#,
        r#"{"requestState":"DONE","resultElementIds":null}"#,
    ] {
        let net = ScriptedTransport::new().route(Method::Get, &url, 200, body);

        assert!(matches!(
            onshape().poll_part_studio(&net, &session(), &translation),
            Err(Error::PartStudioFailed { .. })
        ));
    }
    assert!(matches!(
        onshape().poll_part_studio(
            &ScriptedTransport::new(),
            &session(),
            &Translation {
                source: "tr5".to_owned()
            }
        ),
        Err(Error::NotFound { .. })
    ));
}

fn blob_listing(history: Option<&str>) -> ScriptedTransport {
    let net = ScriptedTransport::new().route(
        Method::Get,
        &elements_url(),
        200,
        r#"[
            {"id":"e4","name":"scan.step","elementType":"BLOB","microversionId":"mv4"},
            {"id":"e5","name":"old.stl","elementType":"BLOB","microversionId":"mv0"},
            {"id":"e6","name":"flat.svg","elementType":"BLOB"},
            {"id":"e7","name":"notes.pdf","elementType":"BLOB","microversionId":"mv4"}
        ]"#,
    );
    match history {
        Some(body) => net.route(
            Method::Get,
            &format!("{API}/documents/d/doc1/w/ws9/documenthistory"),
            200,
            body,
        ),
        None => net,
    }
}

#[test]
fn a_blob_is_dated_by_the_history_entry_of_its_microversion() {
    let net = blob_listing(Some(
        r#"[
            {"microversionId":"mv4","date":"2026-03-02T10:15:30.000Z","description":"Upload","userId":"u1"},
            {"microversionId":"mv3","date":"2026-03-01T08:00:00.000Z"},
            {"microversionId":"mv9","date":"not a date"},
            {"date":"2026-01-01T00:00:00.000Z"}
        ]"#,
    ));

    let folder = onshape()
        .folder(&net, &session(), &workspace_place())
        .unwrap();

    let dated: Vec<_> = folder
        .uploads
        .iter()
        .map(|upload| {
            (
                upload.name.as_str(),
                upload.modified.map(|at| at.to_string()),
            )
        })
        .collect();
    assert_eq!(
        dated,
        [
            ("scan.step", Some("2026-03-02T10:15:30Z".to_owned())),
            ("flat.svg", None),
            ("old.stl", None),
        ]
    );
}

#[test]
fn a_history_that_cannot_be_read_leaves_the_blobs_undated_but_not_the_folder_unlisted() {
    for status in [403, 404, 500] {
        let net = blob_listing(None).route(
            Method::Get,
            &format!("{API}/documents/d/doc1/w/ws9/documenthistory"),
            status,
            "",
        );

        let folder = onshape()
            .folder(&net, &session(), &workspace_place())
            .unwrap();

        assert_eq!(folder.uploads.len(), 3);
        assert!(
            folder
                .uploads
                .iter()
                .all(|upload| upload.modified.is_none())
        );
    }

    let lost = blob_listing(None).route(
        Method::Get,
        &format!("{API}/documents/d/doc1/w/ws9/documenthistory"),
        401,
        "",
    );

    assert!(matches!(
        onshape().folder(&lost, &session(), &workspace_place()),
        Err(Error::SignedOut)
    ));
}

#[test]
fn the_history_is_asked_only_for_a_workspace_that_holds_a_blob_to_date() {
    let history = format!("{API}/documents/d/doc1/w/ws9/documenthistory");
    let without_blobs = ScriptedTransport::new().route(
        Method::Get,
        &elements_url(),
        200,
        r#"[{"id":"e1","name":"Frame","elementType":"PARTSTUDIO"},
            {"id":"e7","name":"notes.pdf","elementType":"BLOB"}]"#,
    );
    let in_version = ScriptedTransport::new().route(
        Method::Get,
        &format!("{API}/documents/d/doc1/v/v1/elements"),
        200,
        r#"[{"id":"e4","name":"scan.step","elementType":"BLOB","microversionId":"mv4"}]"#,
    );
    let version = FolderRef {
        project: ProjectId::new("doc1"),
        folder: Place::version("v1").folder_id(),
    };

    onshape()
        .folder(&without_blobs, &session(), &workspace_place())
        .unwrap();
    let kept = onshape().folder(&in_version, &session(), &version).unwrap();

    assert!(
        without_blobs
            .requests()
            .iter()
            .chain(&in_version.requests())
            .all(|request| request.url != history)
    );
    assert!(kept.uploads.iter().all(|upload| upload.modified.is_none()));
}
