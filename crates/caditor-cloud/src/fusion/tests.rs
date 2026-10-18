use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::{
    encoding::query_pairs,
    fake::ScriptedTransport,
    transport::{Method, TransportError},
};

const NOW: u64 = 1_000;

fn fusion() -> Fusion {
    Fusion::new("client-123")
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
        id: ProjectId::new("a.proj"),
        workspace: WorkspaceId::new("a.hub"),
        name: "Bikes".to_owned(),
    }
}

fn design(id: &str) -> Design {
    Design {
        id: DesignId::new(id),
        project: ProjectId::new("a.proj"),
        name: "Frame".to_owned(),
        modified: None,
    }
}

#[test]
fn the_sign_in_address_carries_pkce_the_redirect_the_scopes_and_a_state() {
    let sign_in = fusion().start_sign_in(&redirect()).unwrap();

    let (base, query) = sign_in.url.split_once('?').unwrap();
    let pairs = query_pairs(query);

    assert_eq!(base, AUTHORIZE);
    assert_eq!(field(&pairs, "response_type").as_deref(), Some("code"));
    assert_eq!(field(&pairs, "client_id").as_deref(), Some("client-123"));
    assert_eq!(
        field(&pairs, "redirect_uri").as_deref(),
        Some("http://localhost:49617/callback")
    );
    assert_eq!(field(&pairs, "scope").as_deref(), Some(SCOPES));
    assert_eq!(field(&pairs, "state"), Some(sign_in.state.clone()));
    assert_eq!(
        field(&pairs, "code_challenge"),
        Some(sign_in.verifier.challenge())
    );
    assert_eq!(
        field(&pairs, "code_challenge_method").as_deref(),
        Some("S256")
    );
    assert!(!sign_in.url.contains(sign_in.verifier.as_str()));
}

#[test]
fn a_build_without_an_app_cannot_start_a_sign_in() {
    let result = Fusion::new("  ").start_sign_in(&redirect());

    assert!(matches!(
        result,
        Err(Error::NoClientId {
            service: Service::Fusion
        })
    ));
}

#[test]
fn finishing_the_sign_in_trades_the_code_and_verifier_for_a_session() {
    let net = ScriptedTransport::new().route(
        Method::Post,
        TOKEN,
        200,
        r#"{"access_token":"a","refresh_token":"r","expires_in":900,"token_type":"Bearer"}"#,
    );
    let provider = fusion();
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
    assert_eq!(
        field(&pairs, "redirect_uri").as_deref(),
        Some("http://localhost:49617/callback")
    );
    assert_eq!(session.access_token, "a");
    assert_eq!(session.refresh_token.as_deref(), Some("r"));
    assert_eq!(session.expires_at, NOW + 900);
}

#[test]
fn a_refresh_keeps_the_old_refresh_token_when_the_answer_has_none() {
    let net = ScriptedTransport::new().route(
        Method::Post,
        TOKEN,
        200,
        r#"{"access_token":"fresh","expires_in":600}"#,
    );

    let renewed = fusion().refresh(&net, &session(), NOW).unwrap();

    let pairs = form_of(net.requests().first().unwrap());
    assert_eq!(
        field(&pairs, "grant_type").as_deref(),
        Some("refresh_token")
    );
    assert_eq!(field(&pairs, "refresh_token").as_deref(), Some("refresh-1"));
    assert_eq!(renewed.access_token, "fresh");
    assert_eq!(renewed.refresh_token.as_deref(), Some("refresh-1"));
    assert_eq!(renewed.expires_at, NOW + 600);
}

#[test]
fn a_refused_refresh_means_signing_in_again() {
    let net =
        ScriptedTransport::new().route(Method::Post, TOKEN, 400, r#"{"error":"invalid_grant"}"#);

    assert!(matches!(
        fusion().refresh(&net, &session(), NOW),
        Err(Error::SignedOut)
    ));
}

#[test]
fn a_session_without_a_refresh_token_cannot_refresh() {
    let net = ScriptedTransport::new();
    let mut session = session();
    session.refresh_token = None;

    assert!(matches!(
        fusion().refresh(&net, &session, NOW),
        Err(Error::SignedOut)
    ));
    assert!(net.requests().is_empty());
}

#[test]
fn signing_out_revokes_the_refresh_token_at_the_service() {
    let net = ScriptedTransport::new().route(Method::Post, REVOKE, 200, "");

    fusion().revoke(&net, &session()).unwrap();

    let pairs = form_of(net.requests().first().unwrap());
    assert_eq!(field(&pairs, "token").as_deref(), Some("refresh-1"));
    assert_eq!(
        field(&pairs, "token_type_hint").as_deref(),
        Some("refresh_token")
    );
    assert_eq!(field(&pairs, "client_id").as_deref(), Some("client-123"));
}

#[test]
fn a_session_without_a_refresh_token_revokes_its_access_token() {
    let net = ScriptedTransport::new().route(Method::Post, REVOKE, 200, "");
    let mut session = session();
    session.refresh_token = None;

    fusion().revoke(&net, &session).unwrap();

    let pairs = form_of(net.requests().first().unwrap());
    assert_eq!(field(&pairs, "token").as_deref(), Some("access-1"));
    assert_eq!(
        field(&pairs, "token_type_hint").as_deref(),
        Some("access_token")
    );
}

#[test]
fn the_account_is_named_by_the_best_field_the_profile_has() {
    let name = |json: &str| {
        let net = ScriptedTransport::new().route(Method::Get, USER_INFO, 200, json);
        fusion().account(&net, &session()).unwrap().name
    };

    assert_eq!(
        name(r#"{"name":"Ada Lovelace","email":"a@x"}"#),
        "Ada Lovelace"
    );
    assert_eq!(
        name(r#"{"given_name":"Ada","family_name":"Lovelace"}"#),
        "Ada Lovelace"
    );
    assert_eq!(name(r#"{"name":" ","email":"a@x"}"#), "a@x");
    assert_eq!(name("{}"), "Autodesk account");
}

#[test]
fn requests_carry_the_access_token_as_a_bearer() {
    let net = ScriptedTransport::new().route(Method::Get, USER_INFO, 200, "{}");

    fusion().account(&net, &session()).unwrap();

    assert_eq!(
        net.requests().first().unwrap().bearer.as_deref(),
        Some("access-1")
    );
}

#[test]
fn hubs_become_workspaces_by_kind_across_pages_and_never_follow_a_foreign_link() {
    let first = format!("{PROJECT_API}/hubs");
    let second = format!("{PROJECT_API}/hubs?page[number]=1");
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &first,
            200,
            &format!(
                r#"{{"data":[
                    {{"type":"hubs","id":"a.team","attributes":{{"name":"Studio","extension":{{"type":"{TEAM_HUB}"}}}}}},
                    {{"type":"hubs","id":"a.me","attributes":{{"name":"Personal","extension":{{"type":"{PERSONAL_HUB}"}}}}}}
                ],"links":{{"next":{{"href":"{second}"}}}}}}"#
            ),
        )
        .route(
            Method::Get,
            &second,
            200,
            r#"{"data":[
                {"type":"hubs","id":"b.acct","attributes":{"name":"Builder","extension":{"type":"hubs:autodesk.bim360:Account"}}},
                {"type":"other","id":"x","attributes":{"name":"Skipped"}}
            ],"links":{"next":{"href":"https://evil.example/steal"}}}"#,
        );

    let workspaces = fusion().workspaces(&net, &session()).unwrap();

    let kinds: Vec<_> = workspaces
        .iter()
        .map(|workspace| {
            (
                workspace.id.as_str(),
                workspace.name.as_str(),
                workspace.kind,
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("a.team", "Studio", WorkspaceKind::Team),
            ("a.me", "Personal", WorkspaceKind::Personal),
            ("b.acct", "Builder", WorkspaceKind::Other),
        ]
    );
    assert_eq!(net.requests().len(), 2);
}

#[test]
fn projects_are_listed_for_the_chosen_hub() {
    let url = format!("{PROJECT_API}/hubs/a.hub/projects");
    let net = ScriptedTransport::new().route(
        Method::Get,
        &url,
        200,
        r#"{"data":[{"type":"projects","id":"a.proj","attributes":{"name":"Bikes"}}]}"#,
    );

    let projects = fusion()
        .projects(&net, &session(), &WorkspaceId::new("a.hub"))
        .unwrap();

    assert_eq!(projects, vec![project()]);
}

fn folder_url(folder: &str) -> String {
    format!("{DATA_API}/projects/a.proj/folders/{folder}/contents")
}

const CONTENTS: &str = r#"{"data":[
    {"type":"folders","id":"urn:f:sub","attributes":{"displayName":"Parts"}},
    {"type":"folders","id":"urn:f:hid","attributes":{"displayName":"Hidden","hidden":true}},
    {"type":"items","id":"urn:i:old","attributes":{"displayName":"Old frame","lastModifiedTime":"2024-01-02T03:04:05.000Z","extension":{"type":"items:autodesk.fusion360:Design"}}},
    {"type":"items","id":"urn:i:new","attributes":{"displayName":"New frame","lastModifiedTime":"2025-06-07T08:09:10.000Z","extension":{"type":"items:autodesk.fusion360:Design"}}},
    {"type":"items","id":"urn:i:pdf","attributes":{"displayName":"Notes.pdf","extension":{"type":"items:autodesk.core:File"}}}
]}"#;

#[test]
fn a_folder_lists_designs_newest_first_and_subfolders_and_leaves_other_files_out() {
    let url = folder_url("urn%3Af%3Atop");
    let net = ScriptedTransport::new().route(Method::Get, &url, 200, CONTENTS);
    let reference = FolderRef {
        project: ProjectId::new("a.proj"),
        folder: FolderId::new("urn:f:top"),
    };

    let folder = fusion().folder(&net, &session(), &reference).unwrap();

    let designs: Vec<_> = folder
        .designs
        .iter()
        .map(|design| (design.id.as_str(), design.name.as_str()))
        .collect();
    assert_eq!(
        designs,
        vec![("urn:i:new", "New frame"), ("urn:i:old", "Old frame")]
    );
    assert_eq!(
        folder
            .designs
            .first()
            .and_then(|design| design.modified)
            .map(|time| time.to_string()),
        Some("2025-06-07T08:09:10Z".to_owned())
    );
    let folders: Vec<_> = folder
        .folders
        .iter()
        .map(|entry| (entry.reference.folder.as_str(), entry.name.as_str()))
        .collect();
    assert_eq!(folders, vec![("urn:f:sub", "Parts")]);
}

#[test]
fn a_project_with_one_top_folder_shows_what_is_in_it() {
    let top = format!("{PROJECT_API}/hubs/a.hub/projects/a.proj/topFolders");
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &top,
            200,
            r#"{"data":[{"type":"folders","id":"urn:f:top","attributes":{"displayName":"Project Files"}}]}"#,
        )
        .route(Method::Get, &folder_url("urn%3Af%3Atop"), 200, CONTENTS);

    let root = fusion().root(&net, &session(), &project()).unwrap();

    assert_eq!(root.designs.len(), 2);
    assert_eq!(root.folders.len(), 1);
}

#[test]
fn a_project_with_several_top_folders_lists_them_as_folders() {
    let top = format!("{PROJECT_API}/hubs/a.hub/projects/a.proj/topFolders");
    let net = ScriptedTransport::new().route(
        Method::Get,
        &top,
        200,
        r#"{"data":[
            {"type":"folders","id":"urn:f:b","attributes":{"displayName":"Plans"}},
            {"type":"folders","id":"urn:f:a","attributes":{"displayName":"Files"}}
        ]}"#,
    );

    let root = fusion().root(&net, &session(), &project()).unwrap();

    assert!(root.designs.is_empty());
    let names: Vec<_> = root
        .folders
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, vec!["Files", "Plans"]);
}

#[test]
fn starting_a_conversion_asks_for_the_newest_version_as_step() {
    let tip = format!("{DATA_API}/projects/a.proj/items/urn%3Ai%3Anew/tip");
    let job = format!("{DERIVATIVE_API}/job");
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &tip,
            200,
            r#"{"data":{"type":"versions","id":"urn:adsk.wipprod:fs.file:vf.X?version=5"}}"#,
        )
        .route(Method::Post, &job, 201, r#"{"result":"created"}"#);

    let translation = fusion()
        .start_conversion(&net, &session(), &design("urn:i:new"))
        .unwrap();

    assert_eq!(
        translation.source,
        base64_url(b"urn:adsk.wipprod:fs.file:vf.X?version=5")
    );
    let requests = net.requests();
    let Body::Json(text) = &requests.get(1).unwrap().body else {
        panic!("the job is JSON");
    };
    let sent: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(sent["input"]["urn"], translation.source.as_str());
    assert_eq!(sent["output"]["formats"][0]["type"], "step");
}

#[test]
fn a_design_that_is_gone_is_reported_as_not_found() {
    let tip = format!("{DATA_API}/projects/a.proj/items/gone/tip");
    let net = ScriptedTransport::new().route(Method::Get, &tip, 404, "{}");

    let result = fusion().start_conversion(&net, &session(), &design("gone"));

    assert!(matches!(
        result,
        Err(Error::NotFound {
            subject: Subject::Design
        })
    ));
}

fn translation() -> Translation {
    Translation {
        source: "dXJu".to_owned(),
    }
}

fn manifest(body: &str) -> ScriptedTransport {
    ScriptedTransport::new().route(
        Method::Get,
        &format!("{DERIVATIVE_API}/dXJu/manifest"),
        200,
        body,
    )
}

#[test]
fn a_conversion_in_progress_reports_its_percentage() {
    let net = manifest(
        r#"{"status":"inprogress","progress":"20% complete","derivatives":[
            {"outputType":"step","status":"inprogress","progress":"45% complete","children":[]}]}"#,
    );

    assert_eq!(
        fusion()
            .poll_conversion(&net, &session(), &translation())
            .unwrap(),
        Conversion::Working { percent: Some(45) }
    );
}

#[test]
fn a_manifest_without_the_step_yet_is_still_working() {
    let net = manifest(
        r#"{"status":"success","progress":"complete","derivatives":[{"outputType":"svf2","status":"success"}]}"#,
    );

    assert_eq!(
        fusion()
            .poll_conversion(&net, &session(), &translation())
            .unwrap(),
        Conversion::Working { percent: Some(100) }
    );
}

#[test]
fn a_finished_conversion_names_the_file_to_download() {
    let net = manifest(
        r#"{"status":"success","derivatives":[{"outputType":"step","status":"success","children":[
            {"urn":"urn:adsk.viewing:fs.file:dXJu/output/Frame.step","role":"step","type":"resource"}]}]}"#,
    );

    assert_eq!(
        fusion()
            .poll_conversion(&net, &session(), &translation())
            .unwrap(),
        Conversion::Ready(Download {
            source: "dXJu".to_owned(),
            artifact: "urn:adsk.viewing:fs.file:dXJu/output/Frame.step".to_owned(),
        })
    );
}

#[test]
fn a_failed_or_empty_conversion_is_an_error() {
    let failed =
        manifest(r#"{"status":"success","derivatives":[{"outputType":"step","status":"failed"}]}"#);
    let whole = manifest(r#"{"status":"failed"}"#);
    let empty = manifest(
        r#"{"status":"success","derivatives":[{"outputType":"step","status":"success","children":[]}]}"#,
    );

    assert!(matches!(
        fusion().poll_conversion(&failed, &session(), &translation()),
        Err(Error::ConversionFailed { .. })
    ));
    assert!(matches!(
        fusion().poll_conversion(&whole, &session(), &translation()),
        Err(Error::ConversionFailed { .. })
    ));
    assert!(matches!(
        fusion().poll_conversion(&empty, &session(), &translation()),
        Err(Error::NoStepFile { .. })
    ));
}

#[test]
fn fetching_streams_the_derivative_named_by_the_manifest_into_the_sink() {
    let artifact = "urn:adsk.viewing:fs.file:dXJu/output/Frame.step";
    let url = format!(
        "{DERIVATIVE_API}/dXJu/manifest/{}",
        percent_encode(artifact)
    );
    let net = ScriptedTransport::new().route(Method::Get, &url, 200, "ISO-10303-21;");
    let download = Download {
        source: "dXJu".to_owned(),
        artifact: artifact.to_owned(),
    };
    let mut sink = Vec::new();
    let stopped = AtomicBool::new(false);

    let bytes = fusion()
        .fetch(&net, &session(), &download, &mut sink, &mut |_, _| {
            !stopped.load(Ordering::Relaxed)
        })
        .unwrap();

    assert_eq!(sink, b"ISO-10303-21;");
    assert_eq!(bytes, 13);
}

#[test]
fn stopping_a_fetch_surfaces_as_a_stop() {
    let artifact = "a/b";
    let url = format!(
        "{DERIVATIVE_API}/dXJu/manifest/{}",
        percent_encode(artifact)
    );
    let net = ScriptedTransport::new().route(Method::Get, &url, 200, "data");
    let download = Download {
        source: "dXJu".to_owned(),
        artifact: artifact.to_owned(),
    };

    let result = fusion().fetch(&net, &session(), &download, &mut Vec::new(), &mut |_, _| {
        false
    });

    assert!(result.unwrap_err().is_stop());
}

#[test]
fn http_statuses_become_the_errors_that_say_what_to_do() {
    let status = |code: u16| {
        let net = ScriptedTransport::new().route(Method::Get, USER_INFO, code, "{}");
        fusion().account(&net, &session()).unwrap_err()
    };

    assert!(matches!(status(401), Error::SignedOut));
    assert!(matches!(status(403), Error::Forbidden { .. }));
    assert!(matches!(status(404), Error::NotFound { .. }));
    assert!(matches!(status(429), Error::RateLimited { .. }));
    assert!(matches!(status(500), Error::Refused { status: 500, .. }));
}

#[test]
fn an_answer_in_the_wrong_shape_is_unreadable_not_a_crash() {
    let net =
        ScriptedTransport::new().route(Method::Get, &format!("{PROJECT_API}/hubs"), 200, "[1,2]");

    assert!(matches!(
        fusion().workspaces(&net, &session()),
        Err(Error::Unreadable {
            subject: Subject::Workspaces,
            ..
        })
    ));
}

#[test]
fn a_network_failure_is_a_transport_error() {
    let net = ScriptedTransport::new().failing(Method::Get, USER_INFO, TransportError::TimedOut);

    assert!(matches!(
        fusion().account(&net, &session()),
        Err(Error::Transport(TransportError::TimedOut))
    ));
}

#[test]
fn percentages_read_from_the_progress_text() {
    assert_eq!(percent_of(Some("complete")), Some(100));
    assert_eq!(percent_of(Some("37% complete")), Some(37));
    assert_eq!(percent_of(Some("pending")), None);
    assert_eq!(percent_of(None), None);
    assert_eq!(percent_of(Some("250% complete")), None);
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const TIP_BODY: &str =
    r#"{"data":{"type":"versions","id":"urn:adsk.wipprod:fs.file:vf.X?version=5"}}"#;

fn thumbnail_address() -> String {
    let urn = base64_url(b"urn:adsk.wipprod:fs.file:vf.X?version=5");
    format!("{DERIVATIVE_API}/{urn}/thumbnail?width=200&height=200")
}

fn tip_route(net: ScriptedTransport) -> ScriptedTransport {
    net.route(
        Method::Get,
        &format!("{DATA_API}/projects/a.proj/items/frame/tip"),
        200,
        TIP_BODY,
    )
}

fn answer_with(status: u16, body: &str) -> ScriptedTransport {
    tip_route(ScriptedTransport::new()).route(Method::Get, &thumbnail_address(), status, body)
}

#[test]
fn a_thumbnail_is_asked_for_the_newest_version_at_a_fixed_size_and_comes_back_as_bytes() {
    let mut picture = PNG_SIGNATURE.to_vec();
    picture.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
    let net = tip_route(ScriptedTransport::new()).route_bytes(
        Method::Get,
        &thumbnail_address(),
        200,
        &picture,
    );

    let found = fusion()
        .thumbnail(&net, &session(), &design("frame"))
        .unwrap();

    assert_eq!(found, Some(picture));
    let requests = net.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests.get(1).unwrap().url, thumbnail_address());
    assert_eq!(requests.get(1).unwrap().bearer.as_deref(), Some("access-1"));
}

#[test]
fn a_design_autodesk_has_not_rendered_yet_has_no_thumbnail_rather_than_an_error() {
    for status in [404, 202] {
        let net = answer_with(status, "{}");

        let found = fusion()
            .thumbnail(&net, &session(), &design("frame"))
            .unwrap();

        assert_eq!(found, None, "status {status}");
    }
}

#[test]
fn an_empty_thumbnail_answer_is_no_thumbnail() {
    let net = answer_with(200, "");

    assert_eq!(
        fusion()
            .thumbnail(&net, &session(), &design("frame"))
            .unwrap(),
        None
    );
}

#[test]
fn a_thumbnail_refused_for_the_session_or_the_hub_is_reported_in_words() {
    assert!(matches!(
        fusion().thumbnail(&answer_with(401, "{}"), &session(), &design("frame")),
        Err(Error::SignedOut)
    ));
    assert!(matches!(
        fusion().thumbnail(&answer_with(403, "{}"), &session(), &design("frame")),
        Err(Error::Forbidden { .. })
    ));
    assert!(matches!(
        fusion().thumbnail(&answer_with(429, "{}"), &session(), &design("frame")),
        Err(Error::RateLimited { .. })
    ));
}

#[test]
fn a_design_that_is_gone_has_no_thumbnail_to_fetch() {
    let tip = format!("{DATA_API}/projects/a.proj/items/frame/tip");
    let net = ScriptedTransport::new().route(Method::Get, &tip, 404, "{}");

    let result = fusion().thumbnail(&net, &session(), &design("frame"));

    assert!(matches!(
        result,
        Err(Error::NotFound {
            subject: Subject::Design
        })
    ));
}
