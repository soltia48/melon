//! End-to-end tests driving the full relay against `felica-rs`'s in-memory card
//! emulator. The test plays the role of the reader-owning *client*: it forwards
//! each command frame the server emits to the emulated card and feeds the card's
//! response back on the next request.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use felica_rs::felica_standard::{
    EmulatedService, EmulatedSystem, FelicaStandardEmulator, ServiceCode,
};

use crate::keystore::KeyStore;
use crate::session::{EncryptionExchangeInput, MutualAuthInput, SessionManager};

const SYSTEM_CODE: u16 = 0x0003;
const SERVICE_CODE: u16 = 0x0048; // random read/write "with key" (requires_key)
const IDM: [u8; 8] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
const PMM: [u8; 8] = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
const K_SYS: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
const K_AREA: [u8; 8] = [0x21, 0x43, 0x65, 0x87, 0xA9, 0xCB, 0xED, 0x0F];
const K_SVC: [u8; 8] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
const ISSUE_ID: [u8; 8] = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33];
const ISSUE_PARAM: [u8; 8] = [0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB];
const BLOCK: [u8; 16] = [
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
];

fn keystore() -> KeyStore {
    let jsonl = format!(
        concat!(
            r#"{{"system_code":"0003","node":"FFFF","algo":"DES","idm":null,"key":"{sys}"}}"#,
            "\n",
            r#"{{"system_code":"0003","node":"0000","algo":"DES","idm":null,"key":"{area}"}}"#,
            "\n",
            r#"{{"system_code":"0003","node":"0048","algo":"DES","idm":null,"key":"{svc}"}}"#,
            "\n",
        ),
        sys = hex::encode(K_SYS),
        area = hex::encode(K_AREA),
        svc = hex::encode(K_SVC),
    );
    KeyStore::from_reader(jsonl.as_bytes()).expect("keys should parse")
}

fn emulated_card() -> FelicaStandardEmulator {
    let mut system = EmulatedSystem::new(SYSTEM_CODE, IDM, PMM).expect("system");
    system.set_system_key(K_SYS);
    system.set_issue_information(ISSUE_ID, ISSUE_PARAM);
    system.root_area_mut().set_key(K_AREA);

    let mut service =
        EmulatedService::with_blocks(ServiceCode::new(SERVICE_CODE), 0x0000, vec![BLOCK]);
    service.set_key(K_SVC);
    system.add_service(service).expect("service fits root area");

    let mut emulator = FelicaStandardEmulator::new();
    emulator.add_system(system);
    emulator
}

/// Relay a hex command frame to the emulated card and return its raw response.
fn relay_to_card(emulator: &mut FelicaStandardEmulator, frame_hex: &str) -> Vec<u8> {
    let frame = hex::decode(frame_hex).expect("frame should be valid hex");
    emulator
        .handle_frame(&frame)
        .expect("emulated card should respond")
}

fn start_input() -> MutualAuthInput {
    MutualAuthInput {
        idm: Some(IDM),
        pmm: Some(PMM),
        system_code: Some(SYSTEM_CODE),
        areas: Some(vec![0x0000]),
        services: Some(vec![SERVICE_CODE]),
        ..Default::default()
    }
}

fn card_input(session_id: &str, card_response: Vec<u8>) -> MutualAuthInput {
    MutualAuthInput {
        session_id: Some(session_id.to_string()),
        card_response: Some(card_response),
        ..Default::default()
    }
}

/// Run the three-step mutual authentication and return the live manager, the
/// emulated card, and the session id.
async fn authenticate(
    allowed: Option<HashSet<u8>>,
) -> (Arc<SessionManager>, FelicaStandardEmulator, String) {
    let manager = SessionManager::new(Arc::new(keystore()), allowed, Duration::from_secs(60), 16);
    let mut card = emulated_card();

    let response = manager
        .handle_mutual_authentication(start_input())
        .await
        .expect("auth start should succeed");
    assert_eq!(response["step"], "auth1");
    assert_eq!(response["session_created"], true);
    assert_eq!(response["command"]["code"], 0x10);
    let session_id = response["session_id"].as_str().unwrap().to_string();
    let card_response = relay_to_card(&mut card, response["command"]["frame"].as_str().unwrap());

    let response = manager
        .handle_mutual_authentication(card_input(&session_id, card_response))
        .await
        .expect("auth step 2 should succeed");
    assert_eq!(response["step"], "auth2");
    assert_eq!(response["command"]["code"], 0x12);
    let card_response = relay_to_card(&mut card, response["command"]["frame"].as_str().unwrap());

    let response = manager
        .handle_mutual_authentication(card_input(&session_id, card_response))
        .await
        .expect("auth completion should succeed");
    assert_eq!(response["step"], "complete");
    assert_eq!(response["result"]["issue_id"], hex::encode(ISSUE_ID));
    assert_eq!(
        response["result"]["issue_parameter"],
        hex::encode(ISSUE_PARAM)
    );

    (manager, card, session_id)
}

#[tokio::test]
async fn full_mutual_authentication_and_secure_read() {
    let (manager, mut card, session_id) = authenticate(None).await;

    // Secure Read payload = [block_count, block-list element (short form)]:
    // read block 0 of service-list index 0 (the authenticated service 0x0048).
    let response = manager
        .handle_encryption_exchange(EncryptionExchangeInput {
            session_id: Some(session_id.clone()),
            cmd_code: Some(0x14),
            payload: Some(vec![0x01, 0x80, 0x00]),
            ..Default::default()
        })
        .await
        .expect("exchange start should succeed");
    assert_eq!(response["command"]["code"], 0x14);
    let card_response = relay_to_card(&mut card, response["command"]["frame"].as_str().unwrap());

    let response = manager
        .handle_encryption_exchange(EncryptionExchangeInput {
            session_id: Some(session_id.clone()),
            card_response: Some(card_response),
            ..Default::default()
        })
        .await
        .expect("exchange completion should succeed");
    let decrypted = hex::decode(response["response"].as_str().unwrap()).unwrap();
    // Read secure response payload: [SF1, SF2, block_count, block data...], followed
    // by DES padding. Like the reference server, the oracle returns the raw padded
    // plaintext (the secure response carries no length field), so we check the prefix.
    let mut expected = vec![0x00, 0x00, 0x01];
    expected.extend_from_slice(&BLOCK);
    assert!(
        decrypted.starts_with(&expected),
        "decrypted {decrypted:02X?} should start with {expected:02X?}"
    );
    assert_eq!(
        decrypted.len() % 8,
        0,
        "secure payload is DES-block aligned"
    );
}

#[tokio::test]
async fn authenticated_idi_is_exposed_and_spend_is_one_shot() {
    let (manager, _card, session_id) = authenticate(None).await;

    // The verified account is (system_code, idm, issue_id) of the emulated card.
    assert_eq!(
        manager.authenticated_account(&session_id),
        Some((SYSTEM_CODE, IDM, ISSUE_ID))
    );

    // The spend capability yields the account exactly once...
    assert_eq!(
        manager.consume_spend(&session_id).unwrap(),
        (SYSTEM_CODE, IDM, ISSUE_ID)
    );
    // ...and refuses a second claim (defeats session_id replay).
    assert_eq!(manager.consume_spend(&session_id).unwrap_err().status, 403);
}

#[tokio::test]
async fn spend_on_unknown_session_is_not_found() {
    let manager = SessionManager::new(Arc::new(keystore()), None, Duration::from_secs(60), 16);
    assert_eq!(manager.authenticated_account("deadbeef"), None);
    assert_eq!(manager.consume_spend("deadbeef").unwrap_err().status, 404);
}

#[tokio::test]
async fn disallowed_command_code_is_forbidden() {
    let allowed: HashSet<u8> = [0x16].into_iter().collect();
    let (manager, _card, session_id) = authenticate(Some(allowed)).await;

    let err = manager
        .handle_encryption_exchange(EncryptionExchangeInput {
            session_id: Some(session_id),
            cmd_code: Some(0x14),
            payload: Some(vec![0x01, 0x80, 0x00]),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(err.status, 403);
}

#[tokio::test]
async fn unknown_session_is_not_found() {
    let manager = SessionManager::new(Arc::new(keystore()), None, Duration::from_secs(60), 16);
    let err = manager
        .handle_encryption_exchange(EncryptionExchangeInput {
            session_id: Some("deadbeef".to_string()),
            cmd_code: Some(0x14),
            payload: Some(vec![0x01, 0x80, 0x00]),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(err.status, 404);
}

#[tokio::test]
async fn mutual_auth_start_requires_idm_and_pmm() {
    let manager = SessionManager::new(Arc::new(keystore()), None, Duration::from_secs(60), 16);
    let err = manager
        .handle_mutual_authentication(MutualAuthInput {
            system_code: Some(SYSTEM_CODE),
            areas: Some(vec![0x0000]),
            services: Some(vec![SERVICE_CODE]),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(err.status, 400);
}
