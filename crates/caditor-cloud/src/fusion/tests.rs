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
    {"type":"items","id":"urn:i:stl","attributes":{"displayName":"Bracket.STL","lastModifiedTime":"2025-01-02T03:04:05.000Z","extension":{"type":"items:autodesk.core:File"}}},
    {"type":"items","id":"urn:i:dxf","attributes":{"displayName":"Plan.dxf","lastModifiedTime":"2025-08-02T03:04:05.000Z","extension":{"type":"items:autodesk.core:File"}}},
    {"type":"items","id":"urn:i:hiddenstep","attributes":{"displayName":"Old.step","hidden":true,"extension":{"type":"items:autodesk.core:File"}}},
    {"type":"items","id":"urn:i:pdf","attributes":{"displayName":"Notes.pdf","extension":{"type":"items:autodesk.core:File"}}}
]}"#;

#[test]
fn a_folder_lists_designs_newest_first_with_subfolders_and_importable_uploads_and_leaves_other_files_out()
 {
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
    let uploads: Vec<_> = folder
        .uploads
        .iter()
        .map(|upload| (upload.id.as_str(), upload.name.as_str(), upload.format))
        .collect();
    assert_eq!(
        uploads,
        vec![
            ("urn:i:dxf", "Plan.dxf", UploadFormat::Dxf),
            ("urn:i:stl", "Bracket.STL", UploadFormat::Stl),
        ]
    );
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
        .start_conversion(&net, &session(), &design("urn:i:new"), None)
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

    let result = fusion().start_conversion(&net, &session(), &design("gone"), None);

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

fn upload() -> Upload {
    Upload {
        id: UploadId::new("urn:i:stl"),
        project: ProjectId::new("a.proj"),
        name: "Bracket.stl".to_owned(),
        format: UploadFormat::Stl,
        modified: None,
    }
}

fn upload_tip() -> String {
    format!("{DATA_API}/projects/a.proj/items/urn%3Ai%3Astl/tip")
}

const UPLOAD_TIP_BODY: &str = r#"{"data":{"type":"versions","id":"urn:adsk.wipprod:fs.file:vf.S?version=2",
    "relationships":{"storage":{"data":{"type":"objects","id":"urn:adsk.objects:os.object:wip.dm.prod/9d4a-b1.stl"},
    "meta":{"link":{"href":"https://developer.api.autodesk.com/data/v1/projects/a.proj/storage/x"}}}}}}"#;

fn signed_download() -> String {
    format!(
        "{STORAGE_API}/buckets/wip.dm.prod/objects/9d4a-b1.stl/signeds3download?minutesExpiration=10"
    )
}

const SIGNED_LINK: &str =
    "https://signed.example.invalid/wip.dm.prod/9d4a-b1.stl?X-Amz-Signature=s";

fn stored_file_net() -> ScriptedTransport {
    ScriptedTransport::new()
        .route(Method::Get, &upload_tip(), 200, UPLOAD_TIP_BODY)
        .route(
            Method::Get,
            &signed_download(),
            200,
            &format!(r#"{{"status":"complete","url":"{SIGNED_LINK}","size":6}}"#),
        )
        .route(Method::Get, SIGNED_LINK, 200, "solid ")
}

#[test]
fn an_uploaded_file_is_downloaded_from_its_storage_object_through_a_signed_link_without_conversion()
{
    let net = stored_file_net();
    let mut sink = Vec::new();

    let bytes = fusion()
        .fetch_upload(&net, &session(), &upload(), &mut sink, &mut |_, _| true)
        .unwrap();

    assert_eq!(sink, b"solid ");
    assert_eq!(bytes, 6);
    let requests = net.requests();
    let urls: Vec<_> = requests
        .iter()
        .map(|request| request.url.as_str())
        .collect();
    assert_eq!(
        urls,
        vec![upload_tip(), signed_download(), SIGNED_LINK.to_owned()]
    );
    assert_eq!(
        requests
            .first()
            .and_then(|request| request.bearer.as_deref()),
        Some("access-1")
    );
    assert_eq!(
        requests
            .get(1)
            .and_then(|request| request.bearer.as_deref()),
        Some("access-1")
    );
    assert_eq!(
        requests
            .get(2)
            .and_then(|request| request.bearer.as_deref()),
        None
    );
    assert!(urls.iter().all(|url| !url.contains(DERIVATIVE_API)));
}

#[test]
fn an_upload_without_a_storage_object_has_nothing_to_download() {
    let bodies = [
        r#"{"data":{"type":"versions","id":"v"}}"#,
        r#"{"data":{"type":"versions","id":"v","relationships":{"storage":{}}}}"#,
        r#"{"data":{"type":"versions","id":"v","relationships":{"storage":{"data":{"id":"urn:adsk.objects:os.object:nobucket"}}}}}"#,
        r#"{"data":{"type":"versions","id":"v","relationships":{"storage":{"data":{"id":"urn:other:thing/x"}}}}}"#,
    ];
    for body in bodies {
        let net = ScriptedTransport::new().route(Method::Get, &upload_tip(), 200, body);

        let result =
            fusion().fetch_upload(&net, &session(), &upload(), &mut Vec::new(), &mut |_, _| {
                true
            });

        assert!(matches!(result, Err(Error::NoStoredFile { .. })), "{body}");
    }
}

#[test]
fn a_signed_link_that_is_missing_or_not_https_is_refused() {
    for answer in [
        r#"{"status":"complete"}"#,
        r#"{"url":"http://plain.example.invalid/x"}"#,
    ] {
        let net = ScriptedTransport::new()
            .route(Method::Get, &upload_tip(), 200, UPLOAD_TIP_BODY)
            .route(Method::Get, &signed_download(), 200, answer);

        let result =
            fusion().fetch_upload(&net, &session(), &upload(), &mut Vec::new(), &mut |_, _| {
                true
            });

        assert!(
            matches!(result, Err(Error::NoStoredFile { .. })),
            "{answer}"
        );
    }
}

#[test]
fn an_uploaded_file_that_is_gone_or_refused_is_reported_in_words() {
    let gone = ScriptedTransport::new().route(Method::Get, &upload_tip(), 404, "{}");
    let refused = ScriptedTransport::new()
        .route(Method::Get, &upload_tip(), 200, UPLOAD_TIP_BODY)
        .route(
            Method::Get,
            &signed_download(),
            200,
            &format!(r#"{{"url":"{SIGNED_LINK}"}}"#),
        )
        .route(Method::Get, SIGNED_LINK, 403, "");
    let sign_in_ended = ScriptedTransport::new().route(Method::Get, &upload_tip(), 401, "{}");

    let run = |net: &ScriptedTransport| {
        fusion()
            .fetch_upload(net, &session(), &upload(), &mut Vec::new(), &mut |_, _| {
                true
            })
            .unwrap_err()
    };

    assert!(matches!(
        run(&gone),
        Error::NotFound {
            subject: Subject::Upload
        }
    ));
    assert!(matches!(run(&refused), Error::Forbidden { .. }));
    assert!(matches!(run(&sign_in_ended), Error::SignedOut));
}

#[test]
fn stopping_an_upload_download_surfaces_as_a_stop() {
    let result = fusion().fetch_upload(
        &stored_file_net(),
        &session(),
        &upload(),
        &mut Vec::new(),
        &mut |_, _| false,
    );

    assert!(result.unwrap_err().is_stop());
}

#[test]
fn formats_are_told_from_the_file_name_and_other_names_are_not_importable() {
    let format = |name: &str| UploadFormat::of_file_name(name);

    assert_eq!(format("a.STP"), Some(UploadFormat::Step));
    assert_eq!(format("a.b.step"), Some(UploadFormat::Step));
    assert_eq!(format("part.3MF"), Some(UploadFormat::ThreeMf));
    assert_eq!(format("part.obj"), Some(UploadFormat::Obj));
    assert_eq!(format("sheet.svg"), Some(UploadFormat::Svg));
    assert_eq!(format("noextension"), None);
    assert_eq!(format("model.f3d"), None);
    assert_eq!(format("notes.pdf"), None);
    assert_eq!(upload().stem(), "Bracket");
}

fn versions_url() -> String {
    format!("{DATA_API}/projects/a.proj/items/urn%3Ai%3Anew/versions")
}

#[test]
fn a_designs_versions_are_listed_newest_first_across_pages() {
    let second =
        format!("{DATA_API}/projects/a.proj/items/urn%3Ai%3Anew/versions?page%5Bnumber%5D=1");
    let net = ScriptedTransport::new()
        .route(
            Method::Get,
            &versions_url(),
            200,
            &format!(
                r#"{{"data":[
                {{"type":"versions","id":"urn:adsk.wipprod:fs.file:vf.X?version=1","attributes":{{"versionNumber":1,"lastModifiedTime":"2024-01-02T03:04:05.000Z"}}}},
                {{"type":"versions","id":"urn:adsk.wipprod:fs.file:vf.X?version=2","attributes":{{"versionNumber":2}}}}],
                "links":{{"next":{{"href":"{second}"}}}}}}"#
            ),
        )
        .route(
            Method::Get,
            &second,
            200,
            r#"{"data":[
                {"type":"versions","id":"urn:adsk.wipprod:fs.file:vf.X?version=3","attributes":{"versionNumber":3,"lastModifiedTime":"2025-06-07T08:09:10.000Z"}},
                {"type":"versions","id":"urn:odd","attributes":{"name":"no number"}}]}"#,
        );

    let versions = fusion()
        .versions(&net, &session(), &design("urn:i:new"))
        .unwrap();

    let numbers: Vec<_> = versions.iter().map(|version| version.number).collect();
    assert_eq!(numbers, vec![3, 2, 1]);
    assert_eq!(
        versions.first().map(|version| version.id.as_str()),
        Some("urn:adsk.wipprod:fs.file:vf.X?version=3")
    );
    assert_eq!(
        versions
            .first()
            .and_then(|version| version.modified)
            .map(|time| time.to_string()),
        Some("2025-06-07T08:09:10Z".to_owned())
    );
    assert_eq!(versions.get(1).and_then(|version| version.modified), None);
}

#[test]
fn a_design_without_versions_listed_has_none_and_a_gone_one_is_not_found() {
    let empty = ScriptedTransport::new().route(Method::Get, &versions_url(), 200, r#"{"data":[]}"#);
    let gone = ScriptedTransport::new().route(Method::Get, &versions_url(), 404, "{}");

    assert!(
        fusion()
            .versions(&empty, &session(), &design("urn:i:new"))
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        fusion().versions(&gone, &session(), &design("urn:i:new")),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn a_chosen_version_is_converted_instead_of_the_newest_and_the_tip_is_not_asked() {
    let job = format!("{DERIVATIVE_API}/job");
    let net = ScriptedTransport::new().route(Method::Post, &job, 201, r#"{"result":"created"}"#);
    let version = Version {
        id: VersionId::new("urn:adsk.wipprod:fs.file:vf.X?version=2"),
        number: 2,
        modified: None,
    };

    let translation = fusion()
        .start_conversion(&net, &session(), &design("urn:i:new"), Some(&version))
        .unwrap();

    assert_eq!(
        translation.source,
        base64_url(b"urn:adsk.wipprod:fs.file:vf.X?version=2")
    );
    let requests = net.requests();
    assert_eq!(requests.len(), 1);
    let Body::Json(text) = &requests.first().unwrap().body else {
        panic!("the job is JSON");
    };
    let sent: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(sent["input"]["urn"], translation.source.as_str());
}

const STORAGE_ID: &str = "urn:adsk.objects:os.object:wip.dm.prod/7c1e-aa.step";
fn top() -> FolderRef {
    FolderRef {
        project: ProjectId::new("a.proj"),
        folder: FolderId::new("urn:f:top"),
    }
}

fn storage_url() -> String {
    format!("{DATA_API}/projects/a.proj/storage")
}

fn signed_upload_base() -> String {
    format!("{STORAGE_API}/buckets/wip.dm.prod/objects/7c1e-aa.step/signeds3upload")
}

fn signed_upload_url(parts: usize) -> String {
    format!(
        "{}?parts={parts}&firstPart=1&minutesExpiration=60",
        signed_upload_base()
    )
}

fn storage_body() -> String {
    format!(
        r#"{{"jsonapi":{{"version":"1.0"}},"data":{{"type":"objects","id":"{STORAGE_ID}",
        "relationships":{{"target":{{"data":{{"type":"folders","id":"urn:f:top"}}}}}}}}}}"#
    )
}

fn upload_links(count: usize) -> String {
    let links: Vec<String> = (1..=count)
        .map(|part| {
            format!("\"https://signed.example.invalid/wip.dm.prod/7c1e-aa.step?part={part}\"")
        })
        .collect();
    format!(
        r#"{{"uploadKey":"key-1","uploadExpiration":"2030-01-01T00:00:00Z","urls":[{}]}}"#,
        links.join(",")
    )
}

fn part_url(part: usize) -> String {
    format!("https://signed.example.invalid/wip.dm.prod/7c1e-aa.step?part={part}")
}

const EMPTY_FOLDER: &str = r#"{"data":[
    {"type":"folders","id":"urn:f:sub","attributes":{"displayName":"Parts"}},
    {"type":"items","id":"urn:i:other","attributes":{"displayName":"Other.step","extension":{"type":"items:autodesk.core:File"}}},
    {"type":"items","id":"urn:i:frame","attributes":{"displayName":"Plate","extension":{"type":"items:autodesk.fusion360:Design"}}}
]}"#;

const HOLDING_FOLDER: &str = r#"{"data":[
    {"type":"items","id":"urn:i:plate","attributes":{"displayName":"plate.STEP","extension":{"type":"items:autodesk.core:File"}}}
]}"#;

fn sending_net(contents: &str, parts: usize) -> ScriptedTransport {
    sending_net_with_links(contents, parts, &upload_links(parts))
}

fn sending_net_with_links(contents: &str, parts: usize, links: &str) -> ScriptedTransport {
    ScriptedTransport::new()
        .route(Method::Get, &folder_url("urn%3Af%3Atop"), 200, contents)
        .route(Method::Post, &storage_url(), 201, &storage_body())
        .route(Method::Get, &signed_upload_url(parts), 200, links)
        .route(Method::Post, &signed_upload_base(), 200, r#"{"size":1}"#)
}

fn json_of(request: &Request) -> serde_json::Value {
    match &request.body {
        Body::JsonApi(text) | Body::Json(text) => serde_json::from_str(text).unwrap(),
        other => panic!("expected JSON, found {other:?}"),
    }
}

#[test]
fn a_new_file_gets_a_storage_object_a_signed_upload_and_a_first_version_item() {
    let items = format!("{DATA_API}/projects/a.proj/items");
    let net = sending_net(EMPTY_FOLDER, 1)
        .route(Method::Put, &part_url(1), 200, "")
        .route(
            Method::Post,
            &items,
            201,
            r#"{"data":{"type":"items","id":"urn:i:made"},"included":[{"type":"versions","id":"urn:v:1","attributes":{"versionNumber":1}}]}"#,
        );
    let top = top();
    let mut seen = Vec::new();

    let sent = fusion()
        .send(
            &net,
            &session(),
            &Outgoing {
                folder: &top,
                name: " plate.step ",
                bytes: b"ISO-10303-21;",
            },
            &mut |sent, total| {
                seen.push((sent, total));
                true
            },
        )
        .unwrap();

    assert_eq!(sent.name, "plate.step");
    assert_eq!(sent.outcome, SendOutcome::Created);
    assert_eq!(seen, vec![(0, Some(13)), (13, Some(13))]);
    let requests = net.requests();
    let urls: Vec<_> = requests
        .iter()
        .map(|request| (request.method, request.url.as_str()))
        .collect();
    assert_eq!(
        urls,
        vec![
            (Method::Get, folder_url("urn%3Af%3Atop").as_str()),
            (Method::Post, storage_url().as_str()),
            (Method::Get, signed_upload_url(1).as_str()),
            (Method::Put, part_url(1).as_str()),
            (Method::Post, signed_upload_base().as_str()),
            (Method::Post, items.as_str()),
        ]
    );
    assert!(
        requests
            .iter()
            .filter(|request| request.method != Method::Put)
            .all(|request| request.bearer.as_deref() == Some("access-1"))
    );
    assert_eq!(requests.get(3).unwrap().bearer, None);
    assert_eq!(
        requests.get(3).unwrap().body,
        Body::Bytes(b"ISO-10303-21;".to_vec())
    );
    assert!(matches!(requests.get(1).unwrap().body, Body::JsonApi(_)));
    assert!(matches!(requests.get(5).unwrap().body, Body::JsonApi(_)));

    let storage = json_of(requests.get(1).unwrap());
    assert_eq!(storage["data"]["type"], "objects");
    assert_eq!(storage["data"]["attributes"]["name"], "plate.step");
    assert_eq!(
        storage["data"]["relationships"]["target"]["data"]["id"],
        "urn:f:top"
    );
    assert_eq!(json_of(requests.get(4).unwrap())["uploadKey"], "key-1");

    let item = json_of(requests.get(5).unwrap());
    assert_eq!(item["data"]["type"], "items");
    assert_eq!(item["data"]["attributes"]["displayName"], "plate.step");
    assert_eq!(
        item["data"]["attributes"]["extension"]["type"],
        "items:autodesk.core:File"
    );
    assert_eq!(
        item["data"]["relationships"]["parent"]["data"]["id"],
        "urn:f:top"
    );
    assert_eq!(item["data"]["relationships"]["tip"]["data"]["id"], "1");
    assert_eq!(item["included"][0]["id"], "1");
    assert_eq!(
        item["included"][0]["attributes"]["extension"]["type"],
        "versions:autodesk.core:File"
    );
    assert_eq!(
        item["included"][0]["relationships"]["storage"]["data"]["id"],
        STORAGE_ID
    );
}

#[test]
fn a_file_of_that_name_already_in_the_folder_gets_a_new_version_instead() {
    let versions = format!("{DATA_API}/projects/a.proj/versions");
    let net = sending_net(HOLDING_FOLDER, 1)
        .route(Method::Put, &part_url(1), 200, "")
        .route(
            Method::Post,
            &versions,
            201,
            r#"{"data":{"type":"versions","id":"urn:v:3","attributes":{"name":"Plate.step","versionNumber":3}}}"#,
        );
    let top = top();

    let sent = fusion()
        .send(
            &net,
            &session(),
            &Outgoing {
                folder: &top,
                name: "Plate.step",
                bytes: b"x",
            },
            &mut |_, _| true,
        )
        .unwrap();

    assert_eq!(sent.outcome, SendOutcome::NewVersion { number: Some(3) });
    let requests = net.requests();
    assert!(
        requests
            .iter()
            .all(|request| request.url != format!("{DATA_API}/projects/a.proj/items"))
    );
    let version = json_of(requests.last().unwrap());
    assert_eq!(version["data"]["type"], "versions");
    assert_eq!(
        version["data"]["attributes"]["extension"]["type"],
        "versions:autodesk.core:File"
    );
    assert_eq!(
        version["data"]["relationships"]["item"]["data"]["id"],
        "urn:i:plate"
    );
    assert_eq!(
        version["data"]["relationships"]["item"]["data"]["type"],
        "items"
    );
    assert_eq!(
        version["data"]["relationships"]["storage"]["data"]["id"],
        STORAGE_ID
    );
}

#[test]
fn a_large_file_goes_up_in_parts_of_at_least_five_mebibytes_with_progress_after_each() {
    let items = format!("{DATA_API}/projects/a.proj/items");
    let bytes = vec![7_u8; 2 * PART_BYTES + 10];
    let net = sending_net(EMPTY_FOLDER, 3)
        .route(Method::Put, &part_url(1), 200, "")
        .route(Method::Put, &part_url(2), 200, "")
        .route(Method::Put, &part_url(3), 200, "")
        .route(
            Method::Post,
            &items,
            201,
            r#"{"data":{"type":"items","id":"urn:i:made"}}"#,
        );
    let top = top();
    let mut seen = Vec::new();

    fusion()
        .send(
            &net,
            &session(),
            &Outgoing {
                folder: &top,
                name: "big.step",
                bytes: &bytes,
            },
            &mut |sent, _| {
                seen.push(sent);
                true
            },
        )
        .unwrap();

    let total = u64::try_from(bytes.len()).unwrap();
    let part = u64::try_from(PART_BYTES).unwrap();
    assert_eq!(seen, vec![0, part, 2 * part, total]);
    let sizes: Vec<usize> = net
        .requests()
        .iter()
        .filter(|request| request.method == Method::Put)
        .map(|request| match &request.body {
            Body::Bytes(part) => part.len(),
            other => panic!("expected bytes, found {other:?}"),
        })
        .collect();
    assert_eq!(sizes, vec![PART_BYTES, PART_BYTES, 10]);
}

#[test]
fn stopping_before_the_first_part_sends_no_bytes_and_makes_no_item() {
    let net = sending_net(EMPTY_FOLDER, 1);
    let top = top();

    let result = fusion().send(
        &net,
        &session(),
        &Outgoing {
            folder: &top,
            name: "plate.step",
            bytes: b"x",
        },
        &mut |_, _| false,
    );

    assert!(matches!(result, Err(Error::Stopped)));
    assert!(
        net.requests()
            .iter()
            .all(|request| request.method != Method::Put
                && !request.url.ends_with("/items")
                && request.url != signed_upload_base())
    );
}

fn send_once(net: &ScriptedTransport) -> Result<Sent, Error> {
    let top = top();
    fusion().send(
        net,
        &session(),
        &Outgoing {
            folder: &top,
            name: "plate.step",
            bytes: b"x",
        },
        &mut |_, _| true,
    )
}

#[test]
fn a_folder_the_user_may_not_write_to_is_refused_in_words_before_any_bytes_go() {
    let net = ScriptedTransport::new()
        .route(Method::Get, &folder_url("urn%3Af%3Atop"), 200, EMPTY_FOLDER)
        .route(Method::Post, &storage_url(), 403, "{}");

    let result = send_once(&net);

    assert!(matches!(result, Err(Error::SendForbidden { .. })));
    assert!(
        net.requests()
            .iter()
            .all(|request| request.method != Method::Put)
    );
}

#[test]
fn a_name_autodesk_will_not_reuse_is_reported_as_taken() {
    let items = format!("{DATA_API}/projects/a.proj/items");
    let net = sending_net(EMPTY_FOLDER, 1)
        .route(Method::Put, &part_url(1), 200, "")
        .route(Method::Post, &items, 409, "{}");

    let result = send_once(&net);

    assert!(matches!(
        result,
        Err(Error::NameTaken { name, .. }) if name == "plate.step"
    ));
}

#[test]
fn a_part_the_storage_refuses_is_an_incomplete_upload_and_no_item_is_made() {
    let net = sending_net(EMPTY_FOLDER, 1).route(Method::Put, &part_url(1), 403, "<Error/>");

    let result = send_once(&net);

    assert!(matches!(result, Err(Error::UploadIncomplete { .. })));
    assert!(
        net.requests()
            .iter()
            .all(|request| request.url != signed_upload_base())
    );
}

#[test]
fn a_signed_upload_with_the_wrong_number_of_links_or_a_plain_http_link_is_unreadable() {
    let wrong_count = sending_net_with_links(EMPTY_FOLDER, 1, &upload_links(2));
    assert!(matches!(
        send_once(&wrong_count),
        Err(Error::Unreadable {
            subject: Subject::SignedUpload,
            ..
        })
    ));

    let plain = sending_net_with_links(
        EMPTY_FOLDER,
        1,
        r#"{"uploadKey":"k","urls":["http://signed.example.invalid/x"]}"#,
    );
    assert!(matches!(
        send_once(&plain),
        Err(Error::Unreadable {
            subject: Subject::SignedUpload,
            ..
        })
    ));
}

#[test]
fn an_expired_sign_in_while_sending_means_signing_in_again() {
    let net = ScriptedTransport::new().route(Method::Get, &folder_url("urn%3Af%3Atop"), 401, "{}");

    assert!(matches!(send_once(&net), Err(Error::SignedOut)));
}

#[test]
fn a_folder_lists_where_files_can_be_put() {
    let net = ScriptedTransport::new().route(
        Method::Get,
        &folder_url("urn%3Af%3Atop"),
        200,
        EMPTY_FOLDER,
    );

    let folder = fusion().folder(&net, &session(), &top()).unwrap();

    assert_eq!(folder.place, Some(top()));
}

#[test]
fn the_sign_in_asks_for_the_scope_that_creates_storage_and_a_refresh_names_none() {
    assert!(SCOPES.split(' ').any(|scope| scope == "data:create"));

    let net = ScriptedTransport::new().route(
        Method::Post,
        TOKEN,
        200,
        r#"{"access_token":"fresh","expires_in":600}"#,
    );
    fusion().refresh(&net, &session(), NOW).unwrap();

    let pairs = form_of(net.requests().first().unwrap());
    assert_eq!(field(&pairs, "scope"), None);
}
