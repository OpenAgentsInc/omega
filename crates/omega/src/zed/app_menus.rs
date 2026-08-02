use app_identity::PRODUCT_NAME;
use gpui::{App, Menu, MenuItem, OsAction};
use omega_actions::Quit;

pub fn app_menus(_cx: &mut App) -> Vec<Menu> {
    vec![
        Menu::new(PRODUCT_NAME).items([
            MenuItem::action("About Omega", omega_actions::About),
            MenuItem::separator(),
            MenuItem::action("Open Settings", omega_actions::OpenSettings),
            MenuItem::action("Open Legacy Settings", omega_actions::OpenLegacySettings),
            #[cfg(target_os = "macos")]
            MenuItem::separator(),
            #[cfg(target_os = "macos")]
            MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
            #[cfg(target_os = "macos")]
            MenuItem::separator(),
            #[cfg(target_os = "macos")]
            MenuItem::action("Hide Omega", super::Hide),
            #[cfg(target_os = "macos")]
            MenuItem::action("Hide Others", super::HideOthers),
            MenuItem::separator(),
            MenuItem::action("Quit Omega", Quit),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", editor::actions::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", editor::actions::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", editor::actions::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", editor::actions::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", editor::actions::Paste, OsAction::Paste),
            MenuItem::os_action(
                "Select All",
                editor::actions::SelectAll,
                OsAction::SelectAll,
            ),
            MenuItem::separator(),
            MenuItem::action("Find", agent_ui::ToggleSearch),
        ]),
        Menu::new("View").items([
            MenuItem::action(
                "Zoom In",
                omega_actions::IncreaseBufferFontSize { persist: false },
            ),
            MenuItem::action(
                "Zoom Out",
                omega_actions::DecreaseBufferFontSize { persist: false },
            ),
            MenuItem::action(
                "Reset Zoom",
                omega_actions::ResetBufferFontSize { persist: false },
            ),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", super::ToggleFullScreen),
            MenuItem::action("Toggle Threads Sidebar", agent_ui::ToggleThreadsSidebar),
            MenuItem::separator(),
            MenuItem::submenu(Menu::new("Workbench").items([
                MenuItem::action("Files", agent_ui::workbench_shell::SelectFiles),
                MenuItem::action("Search", agent_ui::workbench_shell::SelectSearch),
                MenuItem::action("Review", agent_ui::workbench_shell::SelectReview),
                MenuItem::action("Forensics", agent_ui::workbench_shell::SelectForensics),
                MenuItem::action("Git", agent_ui::workbench_shell::SelectGit),
                MenuItem::action("Terminal", agent_ui::workbench_shell::SelectTerminal),
                MenuItem::action("Plan", agent_ui::workbench_shell::SelectPlan),
            ])),
        ]),
        Menu::new("Thread").items([
            MenuItem::action("New Thread", agent_ui::NewThread),
            MenuItem::action("Choose Folder…", workspace::Open::default()),
            MenuItem::separator(),
            MenuItem::action("Sarah voice…", agent_ui::OpenSarahAdmission),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", super::Minimize),
            #[cfg(target_os = "macos")]
            MenuItem::separator(),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Documentation", omega_actions::OpenDocs),
            MenuItem::action("Open Source Licenses", omega_actions::OpenLicenses),
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use gpui::{MenuItem, TestAppContext};

    use super::*;

    fn collect_menu_contract(menu: &Menu, parent: &str, found: &mut Vec<String>) {
        let path = if parent.is_empty() {
            menu.name.to_string()
        } else {
            format!("{parent}/{}", menu.name)
        };
        found.push(format!("menu:{path}"));
        for item in &menu.items {
            match item {
                MenuItem::Separator => found.push(format!("separator:{path}")),
                MenuItem::Submenu(menu) => collect_menu_contract(menu, &path, found),
                MenuItem::SystemMenu(menu) => {
                    found.push(format!("system:{path}/{}", menu.name));
                }
                MenuItem::Action {
                    name,
                    action,
                    disabled,
                    ..
                } => found.push(format!(
                    "action:{path}/{name}={}:{}",
                    action.name(),
                    if *disabled { "disabled" } else { "enabled" }
                )),
            }
        }
    }

    #[gpui::test]
    fn the_application_menu_is_the_approved_six_menu_contract(cx: &mut TestAppContext) {
        let menus = cx.update(app_menus);
        assert_eq!(
            menus
                .iter()
                .map(|menu| menu.name.as_ref())
                .collect::<Vec<_>>(),
            [PRODUCT_NAME, "Edit", "View", "Thread", "Window", "Help"]
        );

        let mut contract = Vec::new();
        for menu in &menus {
            collect_menu_contract(menu, "", &mut contract);
        }

        #[cfg(target_os = "macos")]
        let expected = [
            "menu:Omega",
            "action:Omega/About Omega=omega::About:enabled",
            "separator:Omega",
            "action:Omega/Open Settings=omega::OpenSettings:enabled",
            "action:Omega/Open Legacy Settings=omega::OpenLegacySettings:enabled",
            "separator:Omega",
            "system:Omega/Services",
            "separator:Omega",
            "action:Omega/Hide Omega=omega::Hide:enabled",
            "action:Omega/Hide Others=omega::HideOthers:enabled",
            "separator:Omega",
            "action:Omega/Quit Omega=omega::Quit:enabled",
            "menu:Edit",
            "action:Edit/Undo=editor::Undo:enabled",
            "action:Edit/Redo=editor::Redo:enabled",
            "separator:Edit",
            "action:Edit/Cut=editor::Cut:enabled",
            "action:Edit/Copy=editor::Copy:enabled",
            "action:Edit/Paste=editor::Paste:enabled",
            "action:Edit/Select All=editor::SelectAll:enabled",
            "separator:Edit",
            "action:Edit/Find=agent::ToggleSearch:enabled",
            "menu:View",
            "action:View/Zoom In=omega::IncreaseBufferFontSize:enabled",
            "action:View/Zoom Out=omega::DecreaseBufferFontSize:enabled",
            "action:View/Reset Zoom=omega::ResetBufferFontSize:enabled",
            "separator:View",
            "action:View/Toggle Full Screen=omega::ToggleFullScreen:enabled",
            "action:View/Toggle Threads Sidebar=agent::ToggleThreadsSidebar:enabled",
            "separator:View",
            "menu:View/Workbench",
            "action:View/Workbench/Files=omega_workbench::SelectFiles:enabled",
            "action:View/Workbench/Search=omega_workbench::SelectSearch:enabled",
            "action:View/Workbench/Review=omega_workbench::SelectReview:enabled",
            "action:View/Workbench/Forensics=omega_workbench::SelectForensics:enabled",
            "action:View/Workbench/Git=omega_workbench::SelectGit:enabled",
            "action:View/Workbench/Terminal=omega_workbench::SelectTerminal:enabled",
            "action:View/Workbench/Plan=omega_workbench::SelectPlan:enabled",
            "menu:Thread",
            "action:Thread/New Thread=agent::NewThread:enabled",
            "action:Thread/Choose Folder…=workspace::Open:enabled",
            "separator:Thread",
            "action:Thread/Sarah voice…=agent::OpenSarahAdmission:enabled",
            "menu:Window",
            "action:Window/Minimize=omega::Minimize:enabled",
            "separator:Window",
            "menu:Help",
            "action:Help/Documentation=omega::OpenDocs:enabled",
            "action:Help/Open Source Licenses=omega::OpenLicenses:enabled",
        ];
        #[cfg(not(target_os = "macos"))]
        let expected = [
            "menu:Omega",
            "action:Omega/About Omega=omega::About:enabled",
            "separator:Omega",
            "action:Omega/Open Settings=omega::OpenSettings:enabled",
            "action:Omega/Open Legacy Settings=omega::OpenLegacySettings:enabled",
            "separator:Omega",
            "action:Omega/Quit Omega=omega::Quit:enabled",
            "menu:Edit",
            "action:Edit/Undo=editor::Undo:enabled",
            "action:Edit/Redo=editor::Redo:enabled",
            "separator:Edit",
            "action:Edit/Cut=editor::Cut:enabled",
            "action:Edit/Copy=editor::Copy:enabled",
            "action:Edit/Paste=editor::Paste:enabled",
            "action:Edit/Select All=editor::SelectAll:enabled",
            "separator:Edit",
            "action:Edit/Find=agent::ToggleSearch:enabled",
            "menu:View",
            "action:View/Zoom In=omega::IncreaseBufferFontSize:enabled",
            "action:View/Zoom Out=omega::DecreaseBufferFontSize:enabled",
            "action:View/Reset Zoom=omega::ResetBufferFontSize:enabled",
            "separator:View",
            "action:View/Toggle Full Screen=omega::ToggleFullScreen:enabled",
            "action:View/Toggle Threads Sidebar=agent::ToggleThreadsSidebar:enabled",
            "separator:View",
            "menu:View/Workbench",
            "action:View/Workbench/Files=omega_workbench::SelectFiles:enabled",
            "action:View/Workbench/Search=omega_workbench::SelectSearch:enabled",
            "action:View/Workbench/Review=omega_workbench::SelectReview:enabled",
            "action:View/Workbench/Forensics=omega_workbench::SelectForensics:enabled",
            "action:View/Workbench/Git=omega_workbench::SelectGit:enabled",
            "action:View/Workbench/Terminal=omega_workbench::SelectTerminal:enabled",
            "action:View/Workbench/Plan=omega_workbench::SelectPlan:enabled",
            "menu:Thread",
            "action:Thread/New Thread=agent::NewThread:enabled",
            "action:Thread/Choose Folder…=workspace::Open:enabled",
            "separator:Thread",
            "action:Thread/Sarah voice…=agent::OpenSarahAdmission:enabled",
            "menu:Window",
            "action:Window/Minimize=omega::Minimize:enabled",
            "menu:Help",
            "action:Help/Documentation=omega::OpenDocs:enabled",
            "action:Help/Open Source Licenses=omega::OpenLicenses:enabled",
        ];
        assert_eq!(
            contract,
            expected.map(str::to_string),
            "the recursive application-menu tree changed"
        );

        let joined = contract.join("\n");

        let action_contract = [
            ("About Omega", "omega::About"),
            ("Open Settings", "omega::OpenSettings"),
            ("Open Legacy Settings", "omega::OpenLegacySettings"),
            ("Quit Omega", "omega::Quit"),
            ("Undo", "editor::Undo"),
            ("Redo", "editor::Redo"),
            ("Cut", "editor::Cut"),
            ("Copy", "editor::Copy"),
            ("Paste", "editor::Paste"),
            ("Select All", "editor::SelectAll"),
            ("Find", "agent::ToggleSearch"),
            ("Zoom In", "omega::IncreaseBufferFontSize"),
            ("Zoom Out", "omega::DecreaseBufferFontSize"),
            ("Reset Zoom", "omega::ResetBufferFontSize"),
            ("Toggle Full Screen", "omega::ToggleFullScreen"),
            ("Toggle Threads Sidebar", "agent::ToggleThreadsSidebar"),
            ("Files", "omega_workbench::SelectFiles"),
            ("Search", "omega_workbench::SelectSearch"),
            ("Review", "omega_workbench::SelectReview"),
            ("Forensics", "omega_workbench::SelectForensics"),
            ("Git", "omega_workbench::SelectGit"),
            ("Terminal", "omega_workbench::SelectTerminal"),
            ("Plan", "omega_workbench::SelectPlan"),
            ("New Thread", "agent::NewThread"),
            ("Choose Folder…", "workspace::Open"),
            ("Sarah voice…", "agent::OpenSarahAdmission"),
            ("Documentation", "omega::OpenDocs"),
            ("Open Source Licenses", "omega::OpenLicenses"),
        ];
        for (label, action) in action_contract {
            let needle = format!("/{label}={action}:enabled");
            assert!(
                joined.contains(&needle),
                "menu contract lost {needle}\n{joined}"
            );
            assert!(
                omega_zero_base::admits_action(action),
                "enabled menu action {action} is refused"
            );
        }
    }
}
