// swift-tools-version: 6.0
import PackageDescription

let package = Package(
  name: "clinch-imessage-bridge",
  platforms: [.macOS(.v14)],
  products: [
    .executable(name: "clinch-imessage-bridge", targets: ["ClinchIMessageBridge"]),
  ],
  dependencies: [
    .package(
      url: "https://github.com/openclaw/imsg.git",
      revision: "b5b7464bc748af482bfc3059b28d5dab0395da9e"
    ),
    .package(
      url: "https://github.com/PhoneNumberKit/PhoneNumberKit.git",
      exact: "5.0.4"
    ),
  ],
  targets: [
    .executableTarget(
      name: "ClinchIMessageBridge",
      dependencies: [
        .product(name: "IMsgCore", package: "imsg"),
        .product(name: "PhoneNumberKit", package: "PhoneNumberKit"),
      ]
    ),
    .testTarget(
      name: "ClinchIMessageBridgeTests",
      dependencies: ["ClinchIMessageBridge"]
    ),
  ]
)
