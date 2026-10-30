// swift-tools-version:5.5

import PackageDescription

// The iOS side of the gallery plugin (saving into Photos). Built by Tauri's
// plugin build through swift-rs; `.tauri/tauri-api` is copied in by that build.
let package = Package(
  name: "tauri-plugin-gallery",
  platforms: [
    .iOS(.v15)
  ],
  products: [
    .library(
      name: "tauri-plugin-gallery",
      type: .static,
      targets: ["tauri-plugin-gallery"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-gallery",
      dependencies: [
        .byName(name: "Tauri")
      ],
      path: "Sources")
  ]
)
