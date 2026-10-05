// Small progressive enhancements; the page works without any of this.

const REPO = "parevo/kiyi";

// Reveal sections as they scroll into view.
const io = new IntersectionObserver(
  (entries) => {
    for (const e of entries) {
      if (e.isIntersecting) {
        e.target.classList.add("in");
        io.unobserve(e.target);
      }
    }
  },
  { rootMargin: "0px 0px -8% 0px" },
);
document.querySelectorAll(".reveal").forEach((el) => io.observe(el));

// Point the main button at the right download for this computer.
const ua = navigator.userAgent;
const os = /Windows/.test(ua) ? "windows" : /Mac/.test(ua) ? "mac" : null;
const primary = document.querySelector("[data-primary-label]");
if (os === "mac") primary.textContent = "Download for macOS";
if (os === "windows") primary.textContent = "Download for Windows";
if (os) document.querySelector(`.dl[data-os="${os}"]`)?.classList.add("recommended");

// Link straight to the latest release's files.
fetch(`https://api.github.com/repos/${REPO}/releases/latest`)
  .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
  .then((release) => {
    document.querySelectorAll("[data-version]").forEach((el) => (el.textContent = release.tag_name));
    const find = (suffix) => release.assets.find((a) => a.name.endsWith(suffix))?.browser_download_url;
    document.querySelectorAll("[data-asset]").forEach((a) => {
      const url = find(a.dataset.asset);
      if (url) a.href = url;
    });
    const mainAsset = os === "windows" ? find("x64-setup.exe") : os === "mac" ? find("aarch64.dmg") : null;
    const main = document.querySelector("[data-primary-download]");
    if (mainAsset && os === "windows") main.href = mainAsset;
    // On a Mac we can't tell Apple Silicon from Intel reliably, so send people to the choice.
  })
  .catch(() => {});

fetch(`https://api.github.com/repos/${REPO}`)
  .then((r) => (r.ok ? r.json() : Promise.reject(r.status)))
  .then((repo) => {
    const n = repo.stargazers_count;
    if (n > 0) document.querySelector("[data-stars]").textContent = `${n >= 1000 ? (n / 1000).toFixed(1) + "k" : n} stars`;
  })
  .catch(() => {});

// Cycle example prompts in the AI section, typed out.
const typed = document.querySelector("[data-typed]");
const prompts = [
  "pro customers in germany who spent over 1k",
  "orders refunded last month, newest first",
  "products running low on stock",
  "users who signed up this year but never ordered",
  "müşterileri şehre göre göster",
];
if (typed && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
  let p = 0;
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  (async () => {
    for (;;) {
      await sleep(2600);
      const current = typed.textContent;
      for (let i = current.length; i >= 0; i--) {
        typed.textContent = current.slice(0, i);
        await sleep(14);
      }
      p = (p + 1) % prompts.length;
      for (let i = 0; i <= prompts[p].length; i++) {
        typed.textContent = prompts[p].slice(0, i);
        await sleep(38);
      }
    }
  })();
}
