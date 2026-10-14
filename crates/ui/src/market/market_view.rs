use std::sync::Arc;
use nanoid::nanoid;
use gpui_kit::base::{TreeItem, TreeState, h_flex};
use gpui_kit::component::tree::Tree;
use gpui_kit::component::{IconName, tree::{tree}, list::{ListItem}};
use gpui_kit::{AppContext, Entity, ImageFormat, px};
use gpui_kit::{Context, Image, ImageSource, IntoElement, ParentElement, Render, Styled, Window, base::StyledExt, component::ActiveTheme, div, img};
use noema_config::NoemaConfig;
use permanence::{FunctionalDir, read_functional_dir};
use smap::{ElementKind, SystemMap, read_system_map, validate};

use crate::market::market_factory::{MarketSection, build_market};
use crate::not_logged_in::NOEMA_LOGO;
use crate::FONT_FAMILY;
use crate::smap::smap_utils::read_system_maps;


pub struct MarketView {
    logo: Image,
    sections: Vec<MarketSection>,
    tree_state: Entity<TreeState>,
    system_maps: Vec<SystemMap>,
    config: NoemaConfig,
}


fn construct_system_maps_tree(system_maps: &Vec<SystemMap>) -> Vec<TreeItem>{
    let mut tree_items = vec![];
    for map in system_maps {

        let is_first = tree_items.len() == 0;
        let id = nanoid!();
        let mut tree_item = TreeItem::new(id, &map.name).expanded(is_first);

        let mut temporary_skipped = vec![];
        let mut subsystems = vec![];
        // preprocess
        for child in &map.elements {
            match &child.kind {
                ElementKind::System => {
                    // skip, dont repeat the top level item
                },
                ElementKind::Subsystem => {
                    // find all children of this subsystem
                    subsystems.push(child.clone());
                }
                _ => {
                    // collect so we can build the child/ parent hierarchy
                    temporary_skipped.push(child.clone());
                }

            }

        }


        for sub in subsystems {
            let id = nanoid!();
            let mut sub_tree_item = TreeItem::new(id, &sub.name);
            // find all items that name this item as parent and "drain the temporary_skipped" of it
            let (matched, rest): (Vec<_>, Vec<_>) = temporary_skipped
                .into_iter()
                .partition(|item| item.parent.as_deref() == Some(sub.name.as_str()));
            temporary_skipped = rest;

            for child in matched {
                sub_tree_item = sub_tree_item.child(TreeItem::new(nanoid!(), &child.name));
            }
            tree_item = tree_item.child(sub_tree_item);

            // let sub_children = &temporary_skipped.iter().filter(|item| item.parent == Some(sub.name.clone())).collect::<Vec<_>>();
            // for child in sub_children {
            //     let id = nanoid!();
            //     let tree_child = TreeItem::new(id, &child.name);
            //     sub_tree_item = sub_tree_item.child(tree_child);
            // }
            //
            // tree_item = tree_item.child(sub_tree_item);

        }

        tree_items.push(tree_item);

        for other_item in temporary_skipped.drain(..) {
            tree_items.push(TreeItem::new(nanoid!(), &other_item.name.clone()));
        }

        // should be empty by now
        assert_eq!(temporary_skipped.len(), 0);


    }

    // NOTE:
    // Rules:
    // - [x] Top Entry (system) is irrelevant
    // - [x] First level is Subsystem e.g. Stage1
    // - [ ] kind determines icon e.g. hardware
    // - [x] its a child if it has a parent

    tree_items
}


impl MarketView {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cfg = NoemaConfig::from_home().expect("unable to read config");
        let system_maps = read_system_maps(&cfg);
        // TODO: construct treestate
        let tree_items = construct_system_maps_tree(&system_maps);
        let tree_state = cx.new(|cx| {
            TreeState::new(cx).items(tree_items)
        });

        Self {
            logo: Image::from_bytes(ImageFormat::Png, NOEMA_LOGO.to_vec()),
            sections: build_market(),
            system_maps,
            config: cfg,
            tree_state,
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h_flex()
            .items_center()
            .p_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                img(ImageSource::Image(Arc::new(self.logo.clone())))
                    .h(px(60.))
                    .border_0(),
            )
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> Tree {
        tree(&self.tree_state, |ix, entry, selected, window, cx| {
            let item = entry.item();
            let icon = if !entry.is_folder() {
                IconName::BookOpen
            } else if entry.is_expanded() {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            };

            ListItem::new(ix)
                .selected(selected)
                .pl(px(16.) * entry.depth() + px(12.)) // Indent based on depth
                .child(
                    h_flex()
                        .gap_2()
                        .child(icon)
                        .child(item.label.clone())
                )
                // .on_click(cx.listener(move |_, _, _, _| {
                //     // Handle item click
                // }))
        })

    }
}
impl Render for MarketView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let treeview = self.render_tree(cx);
        div()
            .size_full()
            .v_flex()
            .child(self.render_header(cx))
            .child(
                div()
                    .flex_1()
                    .v_flex()
                    .items_start()
                    .px_4()
                    .pt_8()
                    .gap_y_3()
                    .child(treeview)
            )
    }
}


