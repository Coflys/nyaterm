//! Exercise the production bridge through encoded CLIPRDR PDUs without reading or
//! modifying the host clipboard. The input sink replaces only IronRDP's queue.

use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_cliprdr::backend::ClipboardMessage;
use ironrdp_cliprdr::pdu::{
    Capabilities, ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags,
    ClipboardPdu, ClipboardProtocolVersion, FORMAT_NAME_FILE_LIST, FormatDataRequest, FormatList,
    FormatListResponse, OwnedFormatDataResponse,
};
use ironrdp_core::{decode, encode_vec};
use ironrdp_svc::{SvcMessage, SvcProcessor};
use nyaterm_remote_desktop::RdpControlMessage;

use super::files::{build_local_snapshot, to_filetime};
use super::{ClipboardBridge, ClipboardInput};
use crate::Outbound;

struct ProtocolHarness {
    bridge: Arc<ClipboardBridge>,
    client: CliprdrClient,
    messages: mpsc::Receiver<ClipboardMessage>,
    output: mpsc::Receiver<Outbound>,
}

impl ProtocolHarness {
    fn new() -> Self {
        Self::with_file_capability(false)
    }

    fn new_with_files() -> Self {
        Self::with_file_capability(true)
    }

    fn with_file_capability(file_enabled: bool) -> Self {
        let (output_tx, output) = mpsc::sync_channel(1);
        let bridge = ClipboardBridge::new("clipboard-test".into(), output_tx, file_enabled);
        let (sender, messages) = mpsc::channel();
        bridge.state.lock().unwrap().input = Some(ClipboardInput::Test(sender));
        let client = bridge.cliprdr_client();
        Self {
            bridge,
            client,
            messages,
            output,
        }
    }

    fn receive(&mut self, pdu: ClipboardPdu<'_>) -> Vec<Vec<u8>> {
        let bytes = encode_vec(&pdu).unwrap();
        self.client
            .process(&bytes)
            .unwrap()
            .into_iter()
            .map(|message| message.encode_unframed_pdu().unwrap())
            .collect()
    }

    fn outgoing(&mut self) -> Vec<Vec<u8>> {
        let messages: Vec<SvcMessage> = match self.messages.try_recv().expect("bridge response") {
            ClipboardMessage::SendInitiateCopy(formats) => {
                self.client.initiate_copy(&formats).unwrap().into()
            }
            ClipboardMessage::SendFormatData(response) => {
                self.client.submit_format_data(response).unwrap().into()
            }
            ClipboardMessage::SendInitiatePaste(format) => {
                self.client.initiate_paste(format).unwrap().into()
            }
            _ => panic!("unexpected clipboard message"),
        };
        messages
            .into_iter()
            .map(|message| message.encode_unframed_pdu().unwrap())
            .collect()
    }

    fn connect(&mut self) -> Vec<ClipboardFormat> {
        self.connect_with(
            ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES,
            false,
        )
    }

    /// File capability is negotiated, but `file_available` is cleared after the
    /// handshake so `on_ready` never reads the host clipboard.
    fn connect_files(&mut self) -> Vec<ClipboardFormat> {
        self.connect_with(
            ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
                | ClipboardGeneralCapabilityFlags::STREAM_FILECLIP_ENABLED
                | ClipboardGeneralCapabilityFlags::FILECLIP_NO_FILE_PATHS,
            true,
        )
    }

    fn connect_with(
        &mut self,
        capabilities: ClipboardGeneralCapabilityFlags,
        avoid_host_clipboard: bool,
    ) -> Vec<ClipboardFormat> {
        self.receive(ClipboardPdu::Capabilities(Capabilities::new(
            ClipboardProtocolVersion::V2,
            capabilities,
        )));
        if avoid_host_clipboard {
            self.bridge.file_available.store(false, Ordering::SeqCst);
        }
        self.receive(ClipboardPdu::MonitorReady);
        let initial = self.outgoing();
        assert_eq!(
            initial.len(),
            3,
            "capabilities, temporary directory, format list"
        );
        let ClipboardPdu::FormatList(initial_formats) =
            decode::<ClipboardPdu<'_>>(&initial[2]).unwrap()
        else {
            panic!("expected initial format list");
        };
        let initial_formats = initial_formats.get_formats(true).unwrap();
        self.receive(ClipboardPdu::FormatListResponse(FormatListResponse::Ok));
        let current = self.outgoing();
        assert_eq!(current.len(), 1);
        self.receive(ClipboardPdu::FormatListResponse(FormatListResponse::Ok));
        assert!(
            self.messages.try_recv().is_err(),
            "acknowledgements must not loop"
        );
        if avoid_host_clipboard {
            self.bridge.shutdown.store(true, Ordering::SeqCst);
        }
        initial_formats
    }

    fn request_text(&mut self, expected: &str) {
        self.receive(ClipboardPdu::FormatDataRequest(FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT,
        }));
        let response = self.outgoing();
        assert_eq!(response.len(), 1);
        // Check Windows' wire representation independently of IronRDP's decoder:
        // CB_FORMAT_DATA_RESPONSE, CB_RESPONSE_OK, byte count, UTF-16LE + NUL.
        let expected_payload: Vec<u8> = expected
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(&response[0][..4], &[5, 0, 1, 0]);
        assert_eq!(
            u32::from_le_bytes(response[0][4..8].try_into().unwrap()) as usize,
            expected_payload.len()
        );
        assert_eq!(&response[0][8..], expected_payload);
        let ClipboardPdu::FormatDataResponse(response) =
            decode::<ClipboardPdu<'_>>(&response[0]).unwrap()
        else {
            panic!("expected format data response");
        };
        assert!(!response.is_error());
        assert_eq!(response.to_unicode_string().unwrap(), expected);
    }
}

#[test]
fn local_text_after_handshake_is_advertised_and_served_as_unicode() {
    let mut harness = ProtocolHarness::new();
    assert!(harness.connect().is_empty());
    for text in [
        "hello",
        "中文 🐱\r\nsecond line",
        "",
        "new clipboard contents",
    ] {
        harness.bridge.set_local_text(text.into()).unwrap();
        let advertisement = harness.outgoing();
        assert_eq!(advertisement.len(), 1);
        let ClipboardPdu::FormatList(formats) =
            decode::<ClipboardPdu<'_>>(&advertisement[0]).unwrap()
        else {
            panic!("expected format list");
        };
        let formats = formats.get_formats(true).unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].id(), ClipboardFormatId::CF_UNICODETEXT);
        harness.receive(ClipboardPdu::FormatListResponse(FormatListResponse::Ok));
        harness.request_text(text);
    }
}

#[test]
fn text_cached_before_handshake_is_available_to_the_server() {
    let mut harness = ProtocolHarness::new();
    // A copy may arrive before the helper is connected to IronRDP's input queue.
    let input = harness.bridge.state.lock().unwrap().input.take();
    harness
        .bridge
        .set_local_text("copied before connecting".into())
        .unwrap();
    harness.bridge.state.lock().unwrap().input = input;
    let formats = harness.connect();
    assert_eq!(formats.len(), 1);
    assert_eq!(formats[0].id(), ClipboardFormatId::CF_UNICODETEXT);
    harness.request_text("copied before connecting");
}

#[test]
fn unsupported_format_is_rejected_without_disclosing_text() {
    let mut harness = ProtocolHarness::new();
    harness.connect();
    harness
        .bridge
        .set_local_text("private text".into())
        .unwrap();
    harness.outgoing();
    harness.receive(ClipboardPdu::FormatDataRequest(FormatDataRequest {
        format: ClipboardFormatId::CF_BITMAP,
    }));
    let response = harness.outgoing();
    let ClipboardPdu::FormatDataResponse(response) =
        decode::<ClipboardPdu<'_>>(&response[0]).unwrap()
    else {
        panic!("expected format data response");
    };
    assert!(response.is_error());
}

#[test]
fn remote_copy_is_delivered_over_ipc_and_does_not_disable_later_local_copies() {
    let mut harness = ProtocolHarness::new();
    harness.connect();
    let formats = [ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)];
    let acknowledgement = harness.receive(ClipboardPdu::FormatList(
        FormatList::new_unicode(&formats, true).unwrap(),
    ));
    assert_eq!(acknowledgement.len(), 1);
    assert!(matches!(
        decode::<ClipboardPdu<'_>>(&acknowledgement[0]).unwrap(),
        ClipboardPdu::FormatListResponse(FormatListResponse::Ok)
    ));
    let request = harness.outgoing();
    assert_eq!(request.len(), 1);
    assert!(matches!(
        decode::<ClipboardPdu<'_>>(&request[0]).unwrap(),
        ClipboardPdu::FormatDataRequest(FormatDataRequest {
            format: ClipboardFormatId::CF_UNICODETEXT
        })
    ));
    harness.receive(ClipboardPdu::FormatDataResponse(
        OwnedFormatDataResponse::new_unicode_string("remote text"),
    ));
    let Outbound::Control(RdpControlMessage::Clipboard {
        text, generation, ..
    }) = harness.output.try_recv().unwrap()
    else {
        panic!("expected IPC clipboard event");
    };
    assert_eq!(text, "remote text");
    assert_eq!(generation, 1);
    assert!(
        harness.messages.try_recv().is_err(),
        "remote text must not echo"
    );

    harness
        .bridge
        .set_local_text("new local text".into())
        .unwrap();
    harness.outgoing();
    harness.receive(ClipboardPdu::FormatListResponse(FormatListResponse::Ok));
    harness.request_text("new local text");
}

#[test]
fn local_file_copy_carries_all_three_timestamps_on_the_wire() {
    let root = std::env::temp_dir().join(format!("nyaterm-clip-wire-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("sample.bin");
    std::fs::write(&path, b"clipboard").unwrap();
    let metadata = std::fs::metadata(&path).unwrap();

    let mut harness = ProtocolHarness::new_with_files();
    harness.connect_files();
    let snapshot = build_local_snapshot(std::slice::from_ref(&path)).unwrap();
    let messages: Vec<SvcMessage> = harness
        .client
        .initiate_file_copy(snapshot.descriptors.clone())
        .unwrap()
        .into();
    let encoded: Vec<Vec<u8>> = messages
        .into_iter()
        .map(|message| message.encode_unframed_pdu().unwrap())
        .collect();
    let format_list = encoded
        .iter()
        .find_map(|bytes| match decode::<ClipboardPdu<'_>>(bytes).unwrap() {
            ClipboardPdu::FormatList(list) => Some(list),
            _ => None,
        })
        .expect("file format list");
    let file_format = format_list
        .get_formats(true)
        .unwrap()
        .into_iter()
        .find(|format| {
            format
                .name
                .as_ref()
                .is_some_and(|name| name.value() == FORMAT_NAME_FILE_LIST)
        })
        .expect("FileGroupDescriptorW format")
        .id();

    let responses = harness.receive(ClipboardPdu::FormatDataRequest(FormatDataRequest {
        format: file_format,
    }));
    assert_eq!(responses.len(), 1);
    let ClipboardPdu::FormatDataResponse(response) =
        decode::<ClipboardPdu<'_>>(&responses[0]).unwrap()
    else {
        panic!("expected file list response");
    };
    assert!(!response.is_error());
    let data = response.data();
    assert_eq!(u32::from_le_bytes(data[0..4].try_into().unwrap()), 1);
    let descriptor = &data[4..];
    let flags = u32::from_le_bytes(descriptor[0..4].try_into().unwrap());
    const FD_CREATETIME: u32 = 0x8;
    const FD_ACCESSTIME: u32 = 0x10;
    const FD_WRITESTIME: u32 = 0x20;
    assert_eq!(
        flags & (FD_CREATETIME | FD_ACCESSTIME | FD_WRITESTIME),
        FD_CREATETIME | FD_ACCESSTIME | FD_WRITESTIME
    );
    assert_eq!(
        u64::from_le_bytes(descriptor[40..48].try_into().unwrap()),
        to_filetime(metadata.created().ok()).unwrap()
    );
    assert_eq!(
        u64::from_le_bytes(descriptor[48..56].try_into().unwrap()),
        to_filetime(metadata.accessed().ok()).unwrap()
    );
    assert_eq!(
        u64::from_le_bytes(descriptor[56..64].try_into().unwrap()),
        to_filetime(metadata.modified().ok()).unwrap()
    );

    std::fs::remove_dir_all(root).unwrap();
}
