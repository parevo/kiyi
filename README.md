# Kıyı

AI destekli, hızlı ve sade bir PostgreSQL / MySQL istemcisi. Rust + Tauri 2 + React.

## Geliştirme

```sh
pnpm install
docker compose -f dev/docker-compose.yml up -d --wait   # test veritabanları
pnpm tauri dev
```

Test bağlantıları:

- `postgres://kiyi:kiyi@localhost:55432/shop`
- `mysql://kiyi:kiyi@localhost:53306/shop`

Testler:

```sh
cargo test --workspace                          # birim testleri
KIYI_LIVE=1 cargo test -p kiyi-core --test live # gerçek veritabanlarına karşı sürücü testleri
pnpm typecheck
```

## Yapı

```
crates/kiyi-core/   Sürücüler, bağlantı deposu, keychain. Tauri'den bağımsız.
src-tauri/          Tauri komutları (ince köprü) ve updater.
src/                Arayüz: React + React Aria, CSS Modules, CodeMirror 6, Glide Data Grid.
src/styles/tokens.css  Tüm renkler, boşluklar ve fontlar.
```

## Sürüm ve güncelleme

- `main`'e her push → beta sürümü (`vX.Y.Z-beta.N`), beta kanalındaki kullanıcılara gider.
- `vX.Y.Z` tag'i push'lanınca → stable sürüm, herkese gider.

```sh
git tag v0.1.0 && git push origin v0.1.0
```

Uygulama açılışta ve 4 saatte bir güncelleme kontrol eder. Bulursa arka planda indirir, kullanıcı "Yeniden başlat" deyince kurar.

Gereken GitHub secret'ları: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`.
