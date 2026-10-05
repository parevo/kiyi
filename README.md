# Kiyi

Herkes için veritabanı istemcisi: kayıtları bir tablo gibi gör ve düzenle, tablo oluştur, yapısını değiştir; SQL bilmek gerekmez. Geliştirici modunda her işlemin SQL'i görünür.

PostgreSQL, MySQL, MariaDB ve SQLite destekleniyor; özel ağdaki veritabanlarına SSH tunnel ya da AWS SSM üzerinden bağlanılır. Tablolar CSV/JSON olarak dışa aktarılır, CSV'den içe aktarılır. macOS ve Windows. Yeni veritabanları `crates/kiyi-core/src/catalog.rs` ve bir sürücüyle eklenir. Rust + Tauri 2 + React.

## Geliştirme

```sh
pnpm install
./dev/setup-ssh-key.sh                                  # test bastion'ı için yerel anahtar
docker compose -f dev/docker-compose.yml up -d --wait   # test veritabanları + SSH bastion
pnpm tauri dev
```

Test bağlantıları (tunnel testi için: SSH `127.0.0.1:52222`, kullanıcı/şifre `kiyi`, veritabanı host'u `postgres:5432`):

- `postgres://kiyi:kiyi@localhost:55432/shop`
- `mysql://kiyi:kiyi@localhost:53306/shop`

`shop` uygulamada kurcalamak içindir; canlı testler aynı seed'in kopyası olan `kiyi_test` veritabanını kullanır.

Testler:

```sh
cargo test --workspace                          # birim testleri
KIYI_LIVE=1 cargo test -p kiyi-core --test live # gerçek veritabanlarına karşı sürücü testleri
pnpm typecheck
```

## Arayüzü tarayıcıda çalıştırmak (ekran görüntüleri, uçtan uca testler)

`kiyi-devbridge`, core'u küçük bir HTTP sunucusuyla açar; tarayıcıda açılan arayüz Tauri yerine onunla konuşur (`src/dev/bridge.ts`).

```sh
pnpm dev                                   # Vite :1420
cargo run -p kiyi-devbridge                # köprü :1421
node dev/shoot.mjs /tmp/shots dark         # ana ekranların görüntüleri (WebKit)
node dev/flows.mjs /tmp/flows              # gerçek veritabanında uçtan uca akışlar
```

## AI

Tablolarda "Ask AI", isteği seçilen AI sağlayıcısıyla filtreye çevirir. Ayarlar → AI'dan istenen kadar sağlayıcı eklenir: Anthropic (Messages API) ya da OpenAI uyumlu herhangi bir sunucu (OpenAI, Gemini, OpenRouter, Groq, Mistral, yerelde Ollama / LM Studio, özel adres). Anahtarlar Keychain'de saklanır; yoksa sağlayıcının ortam değişkeni (ör. `ANTHROPIC_API_KEY`) kullanılır. Yalnızca tablo yapısı gönderilir, satırlar asla gönderilmez; AI'ın yazdığı koşul çalışmadan önce tek bir ifade olarak doğrulanır.

## Yapı

```
crates/kiyi-core/   Sürücüler, veritabanı kataloğu, SQL planlayıcıları, bağlantı deposu, keychain. Tauri'den bağımsız.
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
