//! Платформенное хранилище секретов.
//!
//! Выбор реализации происходит в compile-time через cfg:
//!
//! | Платформа       | Реализация                    | Хранилище              |
//! |-----------------|-------------------------------|------------------------|
//! | Windows         | `DesktopKeystore`             | Windows Credential Manager |
//! | macOS           | `DesktopKeystore`             | macOS Keychain         |
//! | Linux           | `DesktopKeystore`             | Secret Service (GNOME) / KWallet |
//! | Android         | `AndroidKeystore`             | Android Keystore System |
//! | iOS             | `IosKeystore`                 | iOS Keychain Services  |
//!
//! В тестах всегда используется `InMemoryKeystore` из `core::ports::keystore`.

#[cfg(target_os = "android")]
mod android;

#[cfg(target_os = "ios")]
mod ios;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod desktop;

// Реэкспортируем нужную реализацию под единым именем
#[cfg(target_os = "android")]
pub use android::AndroidKeystore as PlatformKeystore;

#[cfg(target_os = "ios")]
pub use ios::IosKeystore as PlatformKeystore;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use desktop::DesktopKeystore as PlatformKeystore;
