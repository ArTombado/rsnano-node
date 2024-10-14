use crate::AppViewModel;
use eframe::egui::{self, global_theme_preference_switch, Button, Grid, Label, ScrollArea, Sense};
use egui_extras::{Column, TableBuilder};

pub(crate) struct InsightApp {
    model: AppViewModel,
}

impl InsightApp {
    pub(crate) fn new(model: AppViewModel) -> Self {
        Self { model }
    }
}

impl eframe::App for InsightApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.model.update();
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(1.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.model.can_start_node(), Button::new("Start beta node"))
                    .clicked()
                {
                    self.model.start_beta_node();
                }

                if ui
                    .add_enabled(self.model.can_stop_node(), Button::new("Stop node"))
                    .clicked()
                {
                    self.model.stop_node();
                }
                ui.label(self.model.status());

                let mut checked = self.model.msg_recorder.is_recording();
                ui.checkbox(&mut checked, "capture");
                if checked {
                    self.model.msg_recorder.start_recording()
                } else {
                    self.model.msg_recorder.stop_recording()
                }

                if ui.button("clear").clicked() {
                    self.model.msg_recorder.clear();
                }
            });
            ui.add_space(1.0);
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                global_theme_preference_switch(ui);
                ui.separator();
                ui.label("Messages:");
                ui.label(self.model.messages_sent());
                ui.label("sent");
                ui.add_space(10.0);
                ui.label(self.model.messages_received());
                ui.label("received");
                ui.separator();
                ui.label("Blocks:");
                ui.label("?");
                ui.label("bps");
                ui.add_space(10.0);
                ui.label("?");
                ui.label("cps");
                ui.add_space(10.0);
                ui.label(self.model.block_count());
                ui.label("blocks");
                ui.add_space(10.0);
                ui.label(self.model.cemented_count());
                ui.label("cemented");
            });
        });

        egui::SidePanel::left("overview_panel")
            .default_width(300.0)
            .min_width(300.0)
            .resizable(true)
            .show(ctx, |ui| {
                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(false)
                    .auto_shrink(false)
                    .sense(Sense::click())
                    .column(Column::auto())
                    .column(Column::auto())
                    .column(Column::remainder())
                    .header(20.0, |mut header| {
                        header.col(|ui| {
                            ui.strong("Channel");
                        });
                        header.col(|ui| {
                            ui.strong("Direction");
                        });
                        header.col(|ui| {
                            ui.strong("Message");
                        });
                    })
                    .body(|body| {
                        body.rows(20.0, self.model.message_count(), |mut row| {
                            let Some(row_model) = self.model.get_row(row.index()) else {
                                return;
                            };
                            if row_model.is_selected {
                                row.set_selected(true);
                            }
                            row.col(|ui| {
                                ui.add(Label::new(row_model.channel_id).selectable(false));
                            });
                            row.col(|ui| {
                                ui.add(Label::new(row_model.direction).selectable(false));
                            });
                            row.col(|ui| {
                                ui.add(Label::new(row_model.message).selectable(false));
                            });
                            if row.response().clicked() {
                                self.model.select_message(row.index());
                            }
                        })
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(details) = self.model.selected_message() {
                ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    Grid::new("details_grid").num_columns(2).show(ui, |ui| {
                        ui.label("Channel:");
                        ui.label(details.channel_id);
                        ui.end_row();

                        ui.label("Direction:");
                        ui.label(details.direction);
                        ui.end_row();

                        ui.label("Type:");
                        ui.label(details.message_type);
                        ui.end_row();
                    });

                    ui.add_space(20.0);
                    ui.label(details.message);
                });
            }
        });
        ctx.request_repaint();
    }
}
