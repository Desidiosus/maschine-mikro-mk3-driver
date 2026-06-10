pub mod app;
pub mod message;
pub mod update;
pub mod widget {
    pub mod activity_led;
    pub mod group_box;
    pub mod icon;
    pub mod numeric_field;
    pub mod tabs;
}
pub mod io {
    pub mod connection;
    pub mod subscription;
}
pub mod shell {
    pub mod view;
}
pub mod device {
    pub mod hotspots;
    pub mod labels;
    pub mod view;
}
pub mod prefs {
    pub mod overrides;
    pub mod view;
}
pub mod inspector {
    pub mod view;
    pub mod assign {
        pub mod forms;
        pub mod mapping;
        pub mod multi;
        pub mod numeric;
        pub mod view;
    }
}
