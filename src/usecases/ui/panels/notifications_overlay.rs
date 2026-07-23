//! Toast notification overlay rendering.

use super::super::notifications::notification_color;
use super::super::{NotificationUI, UIActions};

pub(super) fn render_notifications(
    ctx: &egui::Context,
    notifications: &[NotificationUI],
    actions: &mut UIActions,
) {
    if notifications.is_empty() {
        return;
    }

    let screen_rect = ctx.content_rect();
    let toast_width = 360.0;
    let toast_height = 48.0;
    let margin = 12.0;
    let spacing = 6.0;

    for (i, notif) in notifications.iter().enumerate() {
        let x = screen_rect.right() - toast_width - margin;
        let y = screen_rect.top() + margin + (toast_height + spacing) * i as f32;

        let toast_rect =
            egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(toast_width, toast_height));

        let layer_id = egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new(format!("notif_{}", i)),
        );
        let painter = ctx.layer_painter(layer_id);

        // Background
        painter.rect_filled(
            toast_rect,
            6.0,
            egui::Color32::from_rgba_unmultiplied(20, 20, 30, 230),
        );

        // Left accent bar
        let accent_color = notification_color(notif.level);
        let bar_rect = egui::Rect::from_min_size(toast_rect.min, egui::vec2(4.0, toast_height));
        painter.rect_filled(
            bar_rect,
            egui::CornerRadius {
                nw: 6,
                sw: 6,
                ne: 0,
                se: 0,
            },
            accent_color,
        );

        // Level label
        painter.text(
            egui::pos2(toast_rect.left() + 12.0, toast_rect.top() + 8.0),
            egui::Align2::LEFT_TOP,
            notif.level.label(),
            egui::FontId::proportional(10.0),
            accent_color,
        );

        // Message text (truncated)
        let _max_msg_width = toast_width - 50.0;
        let msg = if notif.message.len() > 60 {
            format!("{}…", &notif.message[..59])
        } else {
            notif.message.clone()
        };
        painter.text(
            egui::pos2(toast_rect.left() + 12.0, toast_rect.top() + 22.0),
            egui::Align2::LEFT_TOP,
            &msg,
            egui::FontId::proportional(12.0),
            egui::Color32::from_gray(220),
        );

        // Progress bar (fade out indicator)
        let progress_width = toast_width * (1.0 - notif.progress);
        let progress_rect = egui::Rect::from_min_size(
            egui::pos2(toast_rect.left(), toast_rect.bottom() - 2.0),
            egui::vec2(progress_width, 2.0),
        );
        painter.rect_filled(progress_rect, 0.0, accent_color.linear_multiply(0.5));

        // Dismiss button ("x") — use an Area so it's clickable
        let dismiss_id = egui::Id::new(format!("dismiss_notif_{}", i));
        egui::Area::new(dismiss_id)
            .fixed_pos(egui::pos2(
                toast_rect.right() - 24.0,
                toast_rect.top() + 4.0,
            ))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("x")
                                .size(12.0)
                                .color(egui::Color32::GRAY),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    actions.session.notifications_to_dismiss.push(i);
                }
            });
    }

    // Request repaint to animate progress bars
    ctx.request_repaint();
}
