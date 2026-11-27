use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::Notify;

use super::*;
use crate::channel::ClientError;

fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace()
        .map(|b| u8::from_str_radix(b, 16).unwrap())
        .collect()
}

const TOKEN: Token = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// The resumption test vectors of docs/protocol.md: option values and
/// records.
#[test]
fn wire_format_matches_the_protocol_test_vectors() {
    assert_eq!(resumable_option(0x10000), (2, hex("00 00 01 00")));
    assert_eq!(
        session_option(&Grant {
            token: TOKEN,
            replay: 0x10000,
            received: 0
        }),
        (
            4,
            hex("00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f 00 00 01 00 00 00 00 00 00 00 00 00")
        )
    );
    assert_eq!(
        resume_option(&ResumeRequest {
            token: TOKEN,
            received: 0x0102
        }),
        (
            3,
            hex("00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f 02 01 00 00 00 00 00 00")
        )
    );
    assert_eq!(encode_record(REC_DATA, b"ab"), hex("01 02 00 61 62"));
    assert_eq!(encode_ack(0x0102), hex("02 08 00 02 01 00 00 00 00 00 00"));
    assert_eq!(encode_close(CLOSE_NORMAL, "ok"), hex("03 03 00 00 6f 6b"));
}

/// The handshake test vectors of docs/protocol.md, on the wire.
#[tokio::test]
async fn handshake_vectors() {
    // A hello with RESUMABLE, probe "abcd".
    let (mut c, mut s) = tokio::io::duplex(4096);
    let client = tokio::spawn(async move {
        channel::client_with(
            &mut c,
            4,
            Duration::from_secs(3),
            &vec![resumable_option(0x10000)],
        )
        .await
    });
    let mut head = [0u8; 16];
    s.read_exact(&mut head).await.unwrap();
    assert_eq!(
        head.to_vec(),
        hex("42 4c 53 48 01 00 04 00 06 00 02 04 00 00 01 00")
    );
    let mut probe = [0u8; 4];
    s.read_exact(&mut probe).await.unwrap();
    // The reply with SESSION, then the echo.
    s.write_all(&hex(
        "42 4c 53 48 01 00 04 00 1e 00 04 1c 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f 00 00 01 00 00 00 00 00 00 00 00 00",
    ))
    .await
    .unwrap();
    s.write_all(&probe).await.unwrap();
    let ok = client.await.unwrap().unwrap();
    assert_eq!(
        parse_session(&ok.options),
        Some(Grant {
            token: TOKEN,
            replay: 0x10000,
            received: 0
        })
    );

    // A refusal that says the session cannot be resumed.
    let (mut c, mut s) = tokio::io::duplex(4096);
    tokio::spawn(async move {
        let mut hello = [0u8; 14];
        s.read_exact(&mut hello).await.unwrap();
        s.write_all(&hex(
            "42 4c 53 48 01 01 00 00 0b 00 01 07 65 78 70 69 72 65 64 05 00",
        ))
        .await
        .unwrap();
        std::future::pending::<()>().await;
    });
    match channel::client(&mut c, 4, Duration::from_secs(3)).await {
        Err(ClientError::CannotResume(r)) => assert_eq!(r, "expired"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn option_values_tolerate_trailing_bytes_and_reject_short_ones() {
    let mut v = resume_option(&ResumeRequest {
        token: TOKEN,
        received: 7,
    });
    v.1.extend_from_slice(b"later");
    assert_eq!(parse_resume(&vec![v]).unwrap().received, 7);
    assert_eq!(
        parse_resume(&vec![(channel::OPT_RESUME, vec![0; 23])]),
        None
    );
    assert_eq!(
        parse_session(&vec![(channel::OPT_SESSION, vec![0; 27])]),
        None
    );
    assert_eq!(
        parse_resumable(&vec![(channel::OPT_RESUMABLE, vec![1, 0])]),
        None
    );
    // Tiny replay sizes are raised to the minimum.
    assert_eq!(
        parse_resumable(&vec![resumable_option(1)]),
        Some(MIN_REPLAY)
    );
}

#[test]
fn parser_reassembles_records_and_skips_unknown_types() {
    let mut bytes = encode_record(REC_DATA, b"hello");
    bytes.extend(encode_record(99, b"future"));
    bytes.extend(encode_ack(5));
    bytes.extend(encode_close(CLOSE_TIMEOUT, "idle"));
    let mut p = Parser::default();
    let mut got = Vec::new();
    for b in &bytes {
        p.push(&[*b]);
        while let Some(r) = p.next_record().unwrap() {
            got.push(r);
        }
    }
    assert_eq!(
        got,
        vec![
            Record::Data(b"hello".to_vec()),
            Record::Unknown(99),
            Record::Ack(5),
            Record::Close {
                reason: CLOSE_TIMEOUT,
                message: "idle".into()
            },
        ]
    );
    let mut p = Parser::default();
    p.push(&encode_record(REC_ACK, &[1, 2, 3]));
    assert!(p.next_record().is_err());
}

#[test]
fn rewind_accepts_only_offsets_still_held() {
    let mut s = Stream::new(4096, 4096);
    s.out.extend(&[0u8; 1000]);
    assert!(s.take_data(600).is_some());
    s.ack(200).unwrap();
    assert!(s.ack(2000).is_err(), "ACK beyond what was sent");
    assert_eq!(s.rewind(100), Err("offset already acknowledged"));
    assert_eq!(s.rewind(1001), Err("offset beyond the data sent"));
    s.rewind(450).unwrap();
    assert_eq!((s.acked, s.sent), (450, 450));
    // The end of what was read locally is a valid resume point too.
    s.rewind(1000).unwrap();
    assert_eq!(s.unsent(), 0);
}

// ---------------------------------------------------------------- end to end

const REPLAY: u32 = 8192;

fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9e37_79b9) | 1;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect()
}

/// A server's handling of a new channel: read the hello, then start a
/// resumable session or route a resume to its session.
async fn server_channel(
    mut ch: DuplexStream,
    registry: Registry,
    local: Option<DuplexStream>,
    linger: Duration,
) -> Option<tokio::task::JoinHandle<Summary>> {
    let hello = channel::read_hello(&mut ch).await.unwrap();
    let attach = Attach {
        channel: Box::new(ch),
        hello: hello.clone(),
        keep: Box::new(()),
        peer: "test".into(),
    };
    if let Some(req) = parse_resume(&hello.options) {
        if let Err(a) = registry.route(&req.token, attach) {
            refuse_resume(a, "unknown session").await;
        }
        return None;
    }
    let config = Config {
        linger,
        replay: REPLAY,
    };
    Some(tokio::spawn(serve(
        local.unwrap(),
        attach,
        registry,
        config,
        std::future::pending(),
        |_| {},
    )))
}

/// A client's channel handshake for a resumable session.
async fn open(ch: &mut DuplexStream, resume: Option<ResumeRequest>) -> Result<Grant, ClientError> {
    let mut opts = vec![resumable_option(REPLAY)];
    if let Some(r) = resume {
        opts.push(resume_option(&r));
    }
    let ok = channel::client_with(ch, 64, Duration::from_secs(3), &opts).await?;
    Ok(parse_session(&ok.options).expect("SESSION in the reply"))
}

#[tokio::test]
async fn a_session_survives_a_lost_channel_byte_for_byte() {
    let registry = Registry::default();
    let (server_local, sshd) = tokio::io::duplex(2048);
    let (client_local, app) = tokio::io::duplex(2048);
    let down = pattern(300_000, 1);
    let up = pattern(150_000, 2);

    let (mut c1, s1) = tokio::io::duplex(1024);
    let reg = registry.clone();
    let srv = tokio::spawn(async move {
        server_channel(s1, reg, Some(server_local), Duration::from_secs(30)).await
    });
    let grant = open(&mut c1, None).await.unwrap();
    let _session = srv.await.unwrap().unwrap();
    assert_eq!(registry.len(), 1);

    // Both ends of the SSH stream run for the whole test.
    let (mut sshd_r, mut sshd_w) = tokio::io::split(sshd);
    let down_w = down.clone();
    let sshd_writer = tokio::spawn(async move { sshd_w.write_all(&down_w).await.unwrap() });
    let up_len = up.len();
    let sshd_reader = tokio::spawn(async move {
        let mut got = vec![0u8; up_len];
        sshd_r.read_exact(&mut got).await.unwrap();
        got
    });
    let (mut app_r, mut app_w) = tokio::io::split(app);
    let up_w = up.clone();
    let app_writer = tokio::spawn(async move { app_w.write_all(&up_w).await.unwrap() });
    let cut = Arc::new(Notify::new());
    let cut2 = cut.clone();
    let down_len = down.len();
    let app_reader = tokio::spawn(async move {
        let mut got = vec![0u8; down_len];
        app_r.read_exact(&mut got[..100_000]).await.unwrap();
        cut2.notify_one(); // lose the channel while data flows both ways
        app_r.read_exact(&mut got[100_000..]).await.unwrap();
        got
    });

    // The client side of the session, across two channels.
    let reg = registry.clone();
    let client = tokio::spawn(async move {
        let mut local = client_local;
        let mut st = Stream::new(REPLAY, grant.replay);
        let ended = run_attached(c1, &mut local, &mut st, cut.notified()).await;
        assert!(matches!(ended, Ended::Interrupted(())), "{ended:?}");
        // c1 was dropped by run_attached: the server sees the channel close.
        let (mut c2, s2) = tokio::io::duplex(1024);
        tokio::spawn(async move { server_channel(s2, reg, None, Duration::ZERO).await });
        let g2 = open(
            &mut c2,
            Some(ResumeRequest {
                token: grant.token,
                received: st.received(),
            }),
        )
        .await
        .unwrap();
        assert_ne!(g2.token, grant.token, "tokens rotate");
        st.rewind(g2.received).unwrap();
        st.set_peer_replay(g2.replay);
        run_attached(c2, &mut local, &mut st, std::future::pending::<()>()).await
    });

    let got_down = tokio::time::timeout(Duration::from_secs(20), app_reader)
        .await
        .expect("all server output arrives")
        .unwrap();
    assert!(got_down == down, "server to client stream differs");
    let got_up = tokio::time::timeout(Duration::from_secs(20), sshd_reader)
        .await
        .expect("all client input arrives")
        .unwrap();
    assert!(got_up == up, "client to server stream differs");
    sshd_writer.await.unwrap();
    app_writer.await.unwrap();
    client.abort();
}

#[tokio::test]
async fn sshd_exit_ends_the_session_with_close() {
    let registry = Registry::default();
    let (server_local, mut sshd) = tokio::io::duplex(2048);
    let (mut c, s) = tokio::io::duplex(1024);
    let reg = registry.clone();
    let srv = tokio::spawn(async move {
        server_channel(s, reg, Some(server_local), Duration::from_secs(30)).await
    });
    let grant = open(&mut c, None).await.unwrap();
    let session = srv.await.unwrap().unwrap();
    sshd.write_all(b"logout\r\n").await.unwrap();
    drop(sshd);
    let (mut local, mut app) = tokio::io::duplex(2048);
    let mut st = Stream::new(REPLAY, grant.replay);
    let ended = tokio::time::timeout(
        Duration::from_secs(5),
        run_attached(c, &mut local, &mut st, std::future::pending::<()>()),
    )
    .await
    .unwrap();
    assert!(
        matches!(
            ended,
            Ended::Closed {
                reason: CLOSE_NORMAL,
                ..
            }
        ),
        "{ended:?}"
    );
    let mut got = [0u8; 8];
    app.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"logout\r\n");
    let summary = session.await.unwrap();
    assert_eq!(summary.end, "server ended the session");
    assert!(registry.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_session_not_resumed_in_time_ends_and_its_token_is_refused() {
    let registry = Registry::default();
    let (server_local, _sshd) = tokio::io::duplex(2048);
    let (mut c, s) = tokio::io::duplex(1024);
    let reg = registry.clone();
    let srv = tokio::spawn(async move {
        server_channel(s, reg, Some(server_local), Duration::from_secs(120)).await
    });
    let grant = open(&mut c, None).await.unwrap();
    let session = srv.await.unwrap().unwrap();
    drop(c);
    let summary = session.await.unwrap();
    assert_eq!(summary.end, "not resumed within 120s");
    assert!(registry.is_empty());

    let (mut c2, s2) = tokio::io::duplex(1024);
    let reg = registry.clone();
    tokio::spawn(async move { server_channel(s2, reg, None, Duration::ZERO).await });
    match open(
        &mut c2,
        Some(ResumeRequest {
            token: grant.token,
            received: 0,
        }),
    )
    .await
    {
        Err(ClientError::CannotResume(r)) => assert_eq!(r, "unknown session"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_resume_takes_over_a_channel_the_server_still_thinks_is_up() {
    let registry = Registry::default();
    let (server_local, mut sshd) = tokio::io::duplex(2048);
    let (mut c1, s1) = tokio::io::duplex(1024);
    let reg = registry.clone();
    let srv = tokio::spawn(async move {
        server_channel(s1, reg, Some(server_local), Duration::from_secs(30)).await
    });
    let grant = open(&mut c1, None).await.unwrap();
    let _session = srv.await.unwrap().unwrap();
    // The old channel stays open: the server has not noticed a lost link.
    let (mut c2, s2) = tokio::io::duplex(1024);
    let reg = registry.clone();
    tokio::spawn(async move { server_channel(s2, reg, None, Duration::ZERO).await });
    let g2 = open(
        &mut c2,
        Some(ResumeRequest {
            token: grant.token,
            received: 0,
        }),
    )
    .await
    .unwrap();
    // The server closes the old channel.
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), c1.read_to_end(&mut rest))
        .await
        .expect("old channel closed")
        .unwrap();
    // The new one carries the session.
    let (mut local, mut app) = tokio::io::duplex(2048);
    let mut st = Stream::new(REPLAY, g2.replay);
    st.rewind(g2.received).unwrap();
    let client = tokio::spawn(async move {
        run_attached(c2, &mut local, &mut st, std::future::pending::<()>()).await
    });
    sshd.write_all(b"still here").await.unwrap();
    let mut got = [0u8; 10];
    app.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"still here");
    client.abort();
}

#[tokio::test]
async fn a_server_that_ignores_the_options_answers_without_session() {
    let (mut c, mut s) = tokio::io::duplex(4096);
    tokio::spawn(async move {
        let hello = channel::read_hello(&mut s).await.unwrap();
        channel::accept(&mut s, &hello).await.unwrap();
        std::future::pending::<()>().await;
    });
    let ok = channel::client_with(
        &mut c,
        64,
        Duration::from_secs(3),
        &vec![resumable_option(REPLAY)],
    )
    .await
    .unwrap();
    assert_eq!(parse_session(&ok.options), None);
}

/// A client channel that the test can cut: reads and writes fail once the
/// sender fires, as when a link goes away under a channel.
fn cuttable(ch: DuplexStream) -> (Watched<DuplexStream>, tokio::sync::oneshot::Sender<()>) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    (Watched::new(ch, rx), tx)
}

/// A reconnect that fails `failures` times, then resumes on a new channel.
fn reconnector(
    registry: Registry,
    failures: u32,
) -> impl FnMut(
    ResumeRequest,
) -> std::pin::Pin<Box<dyn Future<Output = Reconnect<Watched<DuplexStream>>> + Send>> {
    let mut tries = 0u32;
    move |req| {
        tries += 1;
        let registry = registry.clone();
        let fail = tries <= failures;
        Box::pin(async move {
            if fail {
                return Reconnect::Failed("no link yet".into());
            }
            let (mut c, s) = tokio::io::duplex(1024);
            tokio::spawn(async move { server_channel(s, registry, None, Duration::ZERO).await });
            match open(&mut c, Some(req)).await {
                Ok(g) => Reconnect::Attached(cuttable(c).0, g),
                Err(ClientError::CannotResume(r)) => Reconnect::CannotResume(r),
                Err(e) => Reconnect::Failed(e.to_string()),
            }
        })
    }
}

#[tokio::test]
async fn run_client_resumes_after_a_lost_channel_and_ends_with_ssh() {
    let registry = Registry::default();
    let (server_local, sshd) = tokio::io::duplex(2048);
    let (mut client_local, app) = tokio::io::duplex(2048);
    let down = pattern(200_000, 3);
    let up = pattern(100_000, 4);

    let (mut c1, s1) = tokio::io::duplex(1024);
    let reg = registry.clone();
    let srv = tokio::spawn(async move {
        server_channel(s1, reg, Some(server_local), Duration::from_secs(30)).await
    });
    let grant = open(&mut c1, None).await.unwrap();
    let session = srv.await.unwrap().unwrap();
    let (c1, cut) = cuttable(c1);

    let (mut sshd_r, mut sshd_w) = tokio::io::split(sshd);
    let down_w = down.clone();
    tokio::spawn(async move { sshd_w.write_all(&down_w).await.unwrap() });
    let up_len = up.len();
    let sshd_reader = tokio::spawn(async move {
        let mut got = vec![0u8; up_len];
        sshd_r.read_exact(&mut got).await.unwrap();
        got
    });
    let (mut app_r, mut app_w) = tokio::io::split(app);
    let up_w = up.clone();
    let app_writer = tokio::spawn(async move {
        app_w.write_all(&up_w).await.unwrap();
        app_w
    });
    let down_len = down.len();
    let app_reader = tokio::spawn(async move {
        let mut got = vec![0u8; down_len];
        app_r.read_exact(&mut got[..50_000]).await.unwrap();
        let _ = cut.send(()); // the link goes away mid-stream
        app_r.read_exact(&mut got[50_000..]).await.unwrap();
        (got, app_r)
    });

    let client = tokio::spawn(async move {
        run_client(
            &mut client_local,
            c1,
            grant,
            ClientConfig {
                replay: REPLAY,
                give_up: Duration::from_secs(30),
            },
            reconnector(registry, 2),
            std::future::pending(),
            |_| {},
        )
        .await
    });

    let (got_down, app_r) = tokio::time::timeout(Duration::from_secs(20), app_reader)
        .await
        .expect("all server output arrives")
        .unwrap();
    assert!(got_down == down, "server to client stream differs");
    let got_up = tokio::time::timeout(Duration::from_secs(20), sshd_reader)
        .await
        .expect("all client input arrives")
        .unwrap();
    assert!(got_up == up, "client to server stream differs");

    // Both ends are done (the fake sshd closes once its stream is out):
    // whichever side closes first, the session ends cleanly.
    let app_w = app_writer.await.unwrap();
    drop((app_r, app_w));
    let summary = tokio::time::timeout(Duration::from_secs(5), client)
        .await
        .expect("client ends")
        .unwrap();
    assert!(
        matches!(
            summary.end,
            ClientEnd::Local
                | ClientEnd::Closed {
                    reason: CLOSE_NORMAL,
                    ..
                }
        ),
        "{:?}",
        summary.end
    );
    assert_eq!(summary.resumes, 1);
    assert_eq!(summary.from_server, down.len() as u64);
    assert_eq!(summary.to_server, up.len() as u64);
    let s = tokio::time::timeout(Duration::from_secs(5), session)
        .await
        .unwrap()
        .unwrap();
    assert!(
        s.end == "server ended the session" || s.end.starts_with("client closed the session"),
        "{}",
        s.end
    );
    assert_eq!(s.resumes, 1);
}

#[tokio::test(start_paused = true)]
async fn run_client_gives_up_when_no_channel_comes_back() {
    let registry = Registry::default();
    let (server_local, _sshd) = tokio::io::duplex(2048);
    let (mut c1, s1) = tokio::io::duplex(1024);
    let reg = registry.clone();
    let srv = tokio::spawn(async move {
        server_channel(s1, reg, Some(server_local), Duration::from_secs(30)).await
    });
    let grant = open(&mut c1, None).await.unwrap();
    let _session = srv.await.unwrap().unwrap();
    let (c1, cut) = cuttable(c1);
    cut.send(()).unwrap();
    let (mut local, _app) = tokio::io::duplex(2048);
    let summary = run_client(
        &mut local,
        c1,
        grant,
        ClientConfig {
            replay: REPLAY,
            give_up: Duration::from_secs(5),
        },
        reconnector(registry, u32::MAX),
        std::future::pending(),
        |_| {},
    )
    .await;
    match summary.end {
        ClientEnd::Lost(why) => assert_eq!(why, "not resumed within 5s"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn run_client_stops_at_a_refused_resume() {
    let registry = Registry::default();
    let (server_local, _sshd) = tokio::io::duplex(2048);
    let (mut c1, s1) = tokio::io::duplex(1024);
    let reg = registry.clone();
    let srv = tokio::spawn(async move {
        server_channel(s1, reg, Some(server_local), Duration::from_secs(1)).await
    });
    let grant = open(&mut c1, None).await.unwrap();
    let session = srv.await.unwrap().unwrap();
    // The channel closes and the server lets the session expire before
    // the client comes back.
    drop(c1);
    let s = session.await.unwrap();
    assert_eq!(s.end, "not resumed within 1s");
    let (c, cut) = cuttable(tokio::io::duplex(1024).0);
    cut.send(()).unwrap();
    let (mut local, _app) = tokio::io::duplex(2048);
    let summary = run_client(
        &mut local,
        c,
        grant,
        ClientConfig {
            replay: REPLAY,
            give_up: Duration::from_secs(60),
        },
        reconnector(registry, 0),
        std::future::pending(),
        |_| {},
    )
    .await;
    match summary.end {
        ClientEnd::Lost(why) => assert_eq!(why, "server cannot resume: unknown session"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn run_client_stop_sends_close() {
    let registry = Registry::default();
    let (server_local, _sshd) = tokio::io::duplex(2048);
    let (mut c1, s1) = tokio::io::duplex(1024);
    let reg = registry.clone();
    let srv = tokio::spawn(async move {
        server_channel(s1, reg, Some(server_local), Duration::from_secs(30)).await
    });
    let grant = open(&mut c1, None).await.unwrap();
    let session = srv.await.unwrap().unwrap();
    let (mut local, _app) = tokio::io::duplex(2048);
    let summary = run_client(
        &mut local,
        cuttable(c1).0,
        grant,
        ClientConfig {
            replay: REPLAY,
            give_up: Duration::from_secs(30),
        },
        reconnector(registry.clone(), 0),
        tokio::time::sleep(Duration::from_millis(50)),
        |_| {},
    )
    .await;
    assert!(
        matches!(summary.end, ClientEnd::Cancelled),
        "{:?}",
        summary.end
    );
    let s = tokio::time::timeout(Duration::from_secs(5), session)
        .await
        .unwrap()
        .unwrap();
    assert!(s.end.starts_with("client closed the session"), "{}", s.end);
    assert!(registry.is_empty());
}
