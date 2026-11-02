use crate::strings::tr;
use gpui_kit::{
    base::{SelectableText, TestSupportExt},
    component::{
        ActiveTheme, WindowExt,
        button::{Button, ButtonVariants},
        dialog::{DialogAction, DialogClose, DialogFooter},
        v_flex,
    },
    *,
};
use patchlook::git::SshConnectionControl;
use patchlook::ssh_auth::{HostKey, SshHostVerification};
use std::sync::Arc;

struct Request {
    key: HostKey,
    reply: flume::Sender<bool>,
    control: SshConnectionControl,
}

/// Owned by the window. Dropping the listener rejects active and queued requests.
pub struct HostVerification {
    sender: flume::Sender<Request>,
    _listener: Task<()>,
}

impl HostVerification {
    pub fn new<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> Self {
        let (sender, receiver) = flume::unbounded::<Request>();
        let listener = cx.spawn_in(window, async move |_, cx| {
            while let Ok(request) = receiver.recv_async().await {
                if request.control.is_disconnected() {
                    let _ = request.reply.send(false);
                    continue;
                }
                let Ok(decision) = cx.update(|window, cx| show(request.key, window, cx)) else {
                    break;
                };
                let accepted = decision.recv_async().await.unwrap_or(false);
                let _ = request.reply.send(accepted);
            }
        });
        Self {
            sender,
            _listener: listener,
        }
    }

    pub fn interaction(
        &self,
        control: SshConnectionControl,
    ) -> anyhow::Result<SshHostVerification> {
        let sender = self.sender.clone();
        Ok(SshHostVerification::new(
            std::env::current_exe()?,
            move |key| {
                if control.is_disconnected() {
                    return false;
                }
                let (reply, response) = flume::bounded(1);
                sender
                    .send(Request {
                        key,
                        reply,
                        control: control.clone(),
                    })
                    .is_ok()
                    && response.recv().unwrap_or(false)
            },
        ))
    }
}

struct Decision(flume::Sender<bool>);
impl Decision {
    fn answer(&self, accepted: bool) {
        let _ = self.0.try_send(accepted);
    }
}
impl Drop for Decision {
    fn drop(&mut self) {
        self.answer(false);
    }
}

fn show(key: HostKey, window: &mut Window, cx: &mut App) -> flume::Receiver<bool> {
    let (sender, receiver) = flume::bounded(1);
    let decision = Arc::new(Decision(sender));
    window.open_dialog(cx, move |dialog, _, cx| {
        let accept = decision.clone();
        let cancel = decision.clone();
        let close = decision.clone();
        dialog
            .title(tr("ssh_verify_title"))
            .w(px(540.))
            .overlay_closable(false)
            .child(
                v_flex()
                    .id("ssh-host-verification")
                    .test_support()
                    .gap_3()
                    .text_sm()
                    .child(tr("ssh_verify_description"))
                    .child(
                        v_flex()
                            .gap_1()
                            .child(tr("ssh_host"))
                            .child(SelectableText::new("ssh-verify-host", key.host.clone())),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(tr("ssh_key_type"))
                            .child(key.algorithm.clone()),
                    )
                    .child(v_flex().gap_1().child(tr("ssh_fingerprint")).child(
                        SelectableText::new("ssh-verify-fingerprint", key.fingerprint.clone()),
                    ))
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("ssh_verify_remember")),
                    ),
            )
            .footer(
                DialogFooter::new()
                    .child(
                        DialogClose::new().child(
                            Button::new("ssh-trust-cancel")
                                .label(tr("cancel"))
                                .outline(),
                        ),
                    )
                    .child(
                        DialogAction::new().child(
                            Button::new("ssh-trust-accept")
                                .label(tr("ssh_trust_connect"))
                                .primary(),
                        ),
                    ),
            )
            .on_ok(move |_, _, _| {
                accept.answer(true);
                true
            })
            .on_cancel(move |_, _, _| {
                cancel.answer(false);
                true
            })
            .on_close(move |_, _, _| close.answer(false))
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use gpui_kit::test::TestWindowExt;

    #[gpui_kit::test]
    fn host_confirmation_requires_explicit_approval_and_cancel_declines(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, _) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Empty)).unwrap()
        });
        for accept in [false, true] {
            let response = cx
                .update_window(handle, |_, window, cx| {
                    show(
                        HostKey {
                            host: "[127.0.0.1]:2222".into(),
                            algorithm: "ED25519".into(),
                            fingerprint: format!("SHA256:{}", "A".repeat(43)),
                        },
                        window,
                        cx,
                    )
                })
                .unwrap();
            cx.run_until_parked();
            assert!(response.try_recv().is_err());
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.render_frame(cx);
                assert!(window.find("ssh-host-verification").visible());
                if accept {
                    window.click("ssh-trust-accept", cx);
                } else {
                    window.press("escape", cx);
                }
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(response.recv().unwrap(), accept);
        }
    }

    #[gpui_kit::test]
    fn dropping_window_listener_rejects_active_and_queued_requests(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
        });
        let (handle, owner) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Empty)).unwrap()
        });
        let verifier = cx
            .update_window(handle, |_, window, cx| {
                owner.update(cx, |_, cx| HostVerification::new(window, cx))
            })
            .unwrap();
        let responses = (0..3)
            .map(|index| {
                let (reply, response) = flume::bounded(1);
                let control = SshConnectionControl::default();
                if index == 0 {
                    control.disconnect();
                }
                verifier
                    .sender
                    .send(Request {
                        key: HostKey {
                            host: "server".into(),
                            algorithm: "ED25519".into(),
                            fingerprint: format!("SHA256:{}", "A".repeat(43)),
                        },
                        reply,
                        control,
                    })
                    .unwrap();
                response
            })
            .collect::<Vec<_>>();
        cx.run_until_parked();
        assert_eq!(responses[0].try_recv(), Ok(false));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.render_frame(cx);
            assert!(window.find("ssh-host-verification").visible());
        })
        .unwrap();
        drop(verifier);
        cx.run_until_parked();
        for response in responses.into_iter().skip(1) {
            assert!(matches!(
                response.try_recv(),
                Err(flume::TryRecvError::Disconnected)
            ));
        }
        cx.update_window(handle, |_, window, cx| window.close_all_dialogs(cx))
            .unwrap();
    }

    struct Empty;
    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full()
        }
    }
}
