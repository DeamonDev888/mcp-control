/* Code d'accès API : demandé à l'utilisateur (dialogue de déverrouillage) puis
   gardé en localStorage — JAMAIS injecté par le serveur dans la page (audit
   2026-10-24 : quiconque joint le port lisait le code sur / et contournait
   toute l'auth). En bind loopback aucun code n'est requis : le dialogue
   n'apparaît jamais. Chaque requête /api/ présente le code (X-MCC-Access) ;
   un 401 rouvre le dialogue (code tourné ou faux). */
const accessCode = () => { try { return localStorage.getItem('mcc.access') || ''; } catch (e) { return ''; } };
const MCC_VERSION = '__MCC_VERSION__';
let activeView = 'dashboard';
let explorerTab = 'mcp'; // 'mcp' | 'swarm'
const SWARM_CLASS_SLUGS = new Set([
  'coordinator', 'coder', 'explorer', 'verifier', 'monitor',
  'browser', 'discord', 'google', 'whatsapp', 'x', 'custom',
]);
function swarmLogoClassFor(name, kind, cls) {
  if (kind === 'hive') return 'logo';
  if (kind === 'instance') return 'coordinator';
  const c = String(cls || '').trim().toLowerCase();
  if (c && SWARM_CLASS_SLUGS.has(c)) return c;
  const n = String(name || '').toLowerCase();
  if (/coord|orch|lead|boss|hive|swarm-control/.test(n)) return 'coordinator';
  if (/cod|dev|eng|build|rust|front|back|full/.test(n)) return 'coder';
  if (/explor|research|scout|search|analy/.test(n)) return 'explorer';
  if (/verif|review|audit|test|qa|sec/.test(n)) return 'verifier';
  if (/monit|ops|watch|health|alert/.test(n)) return 'monitor';
  if (/brows|web|chrome|play/.test(n)) return 'browser';
  if (/discord/.test(n)) return 'discord';
  if (/google|gmail|drive|cal|gws/.test(n)) return 'google';
  if (/whatsapp|wa/.test(n)) return 'whatsapp';
  if (/(^|[-_])x($|[-_])|twitter/.test(n)) return 'x';
  return 'custom';
}
function swarmLogoUrl(name, kind, cls) {
  const slug = swarmLogoClassFor(name, kind, cls);
  const rev = (typeof state !== 'undefined' && state && state.swarm_logos_rev)
    ? (MCC_VERSION + '-' + state.swarm_logos_rev)
    : MCC_VERSION;
  return `/swarm-logo/${slug}.svg?v=${encodeURIComponent(rev)}`;
}

/* ===== Thème sombre/clair (⚙ Réglages) =====
   Préférence locale (mcc.theme) appliquée AVANT le premier rendu par le
   script inline anti-flash du <body> — ici on bascule et on marque l'actif. */
// Applique le thème SANS toucher à la préférence enregistrée.
// applyTheme est aussi appelé à l'ouverture des Réglages (pour aligner les
// boutons) : écrire ici écrasait le choix de l'utilisateur — le thème clair
// ne tenait jamais (il revenait à « sombre » à la réouverture des Réglages).
function applyTheme(t) {
  document.documentElement.dataset.theme = t === 'light' ? 'light' : 'dark';
  const light = document.documentElement.dataset.theme === 'light';
  const dk = document.getElementById('tDark');
  const lt = document.getElementById('tLight');
  if (dk) { dk.classList.toggle('sel', !light); dk.setAttribute('aria-pressed', !light ? 'true' : 'false'); }
  if (lt) { lt.classList.toggle('sel', light); lt.setAttribute('aria-pressed', light ? 'true' : 'false'); }
}
// Choix EXPLICITE de l'utilisateur : applique ET enregistre.
function applyThemeAndSave(t) {
  applyTheme(t);
  try {
    localStorage.setItem('mcc.theme', document.documentElement.dataset.theme);
    localStorage.setItem('mcc-theme', document.documentElement.dataset.theme);
  } catch (e) {}
}

const TEXT_COLORS = ['default', 'white', 'warm', 'amber', 'emerald', 'cyan', 'blue', 'violet', 'rose', 'lime'];
function normalizeTextBright(v) {
  const n = parseInt(v, 10);
  if (!Number.isFinite(n)) return 100;
  return Math.max(60, Math.min(140, Math.round(n / 5) * 5));
}
function applyTextStyle(color, bright) {
  const c = TEXT_COLORS.includes(color) ? color : 'default';
  const b = normalizeTextBright(bright);
  const ds = document.documentElement.dataset;
  if (c === 'default') delete ds.textColor; else ds.textColor = c;
  if (b === 100) delete ds.textBright; else ds.textBright = String(b);
  const rng = document.getElementById('sTextBright');
  if (rng && rng.value !== String(b)) rng.value = String(b);
  const val = document.getElementById('sTextBrightVal');
  if (val) val.textContent = b + ' %';
  const rst = document.getElementById('btnTextReset');
  if (rst) rst.disabled = (c === 'default' && b === 100);
  document.querySelectorAll('.txt-sw-btn').forEach(btn => {
    const sel = btn.dataset.color === c;
    btn.classList.toggle('sel', sel);
    btn.setAttribute('aria-pressed', sel ? 'true' : 'false');
  });
}
function applyTextColorAndSave(color) {
  const b = document.documentElement.dataset.textBright || '100';
  applyTextStyle(color, b);
  const saved = document.documentElement.dataset.textColor || 'default';
  try { localStorage.setItem('mcc.textColor', saved); } catch (e) {}
}
function applyTextBrightAndSave(val) {
  const c = document.documentElement.dataset.textColor || 'default';
  applyTextStyle(c, val);
  const saved = document.documentElement.dataset.textBright || '100';
  try { localStorage.setItem('mcc.textBright', saved); } catch (e) {}
}
function resetTextStyle() {
  applyTextStyle('default', 100);
  try {
    localStorage.removeItem('mcc.textColor');
    localStorage.removeItem('mcc.textBright');
  } catch (e) {}
}

/* ===== Modale de confirmation accessible (<dialog id="confirmDlg">) =====
   Remplace tous les alert(), confirm() et prompt() natifs : promesse async,
   pile de focus pour modales imbriquées, annulation propre sur Échap/backdrop. */
let _confirmResolve = null;
const _focusStack = [];
const isFocusableVisible = el => !!(el && el.isConnected && !el.hidden && !el.disabled
  && (!el.closest || !el.closest('[hidden]')) && typeof el.focus === 'function');
function rememberFocus() {
  const el = document.activeElement;
  if (el && el !== document.body && el !== document.documentElement) {
    _focusStack.push(el);
  }
}
function restoreFocus() {
  while (_focusStack.length) {
    const el = _focusStack.pop();
    if (isFocusableVisible(el)) {
      try { el.focus(); return; } catch (e) {}
    }
  }
}
function askConfirm(opts) {
  const o = typeof opts === 'string' ? {msg: opts} : (opts || {});
  const dlg = document.getElementById('confirmDlg');
  if (!dlg) return Promise.resolve(false);
  if (_confirmResolve) {
    const old = _confirmResolve;
    _confirmResolve = null;
    old.resolve(false);
  }
  if (!dlg.open) rememberFocus();
  document.getElementById('confirmDlgTitle').textContent = o.title || 'Confirmation';
  document.getElementById('confirmDlgMsg').textContent = o.msg || '';
  const det = document.getElementById('confirmDlgDetail');
  if (o.detail) { det.textContent = o.detail; det.hidden = false; }
  else { det.textContent = ''; det.hidden = true; }
  const inpBox = document.getElementById('confirmDlgInputs');
  if (Array.isArray(o.inputs) && o.inputs.length) {
    inpBox.hidden = false;
    inpBox.innerHTML = o.inputs.map(f =>
      `<div><label for="cdlg_${esc(f.id)}">${esc(f.label)}</label><input id="cdlg_${esc(f.id)}" type="${esc(f.type || 'text')}" placeholder="${esc(f.placeholder || '')}" value="${esc(f.value || '')}"></div>`
    ).join('');
  } else {
    inpBox.hidden = true;
    inpBox.innerHTML = '';
  }
  const btnCancel = document.getElementById('confirmDlgCancel');
  const btnOk = document.getElementById('confirmDlgOk');
  btnCancel.textContent = o.cancelLabel || 'Annuler';
  btnOk.textContent = o.okLabel || 'Confirmer';
  btnOk.classList.toggle('danger', !!o.danger);
  btnOk.classList.toggle('primary', !o.danger);
  if (!dlg.open) dlg.showModal();
  if (Array.isArray(o.inputs) && o.inputs.length) {
    const first = document.getElementById('cdlg_' + o.inputs[0].id);
    if (first) first.focus();
  } else {
    btnCancel.focus();
  }
  return new Promise(resolve => { _confirmResolve = {resolve, inputs: o.inputs || null}; });
}
function finishConfirmDlg(accepted) {
  const dlg = document.getElementById('confirmDlg');
  const pending = _confirmResolve;
  _confirmResolve = null;
  let val = accepted;
  if (accepted && pending && pending.inputs) {
    val = {};
    pending.inputs.forEach(f => {
      const el = document.getElementById('cdlg_' + f.id);
      val[f.id] = el ? el.value.trim() : '';
    });
  }
  if (dlg && dlg.open) dlg.close();
  restoreFocus();
  if (pending) pending.resolve(val);
}
function confirmDlgOk() {
  finishConfirmDlg(true);
}
function confirmDlgCancel() {
  finishConfirmDlg(false);
}
const _mccFetch = window.fetch.bind(window);

/* Session locale (docs/auth.md §6, lot S1) : le jeton d'amorçage arrive dans le
   FRAGMENT de l'URL — jamais envoyé au serveur, jamais dans Referer, jamais
   dans mcc.log. On l'échange UNE fois contre le cookie `mcc_session` ET un
   secret de session gardé UNIQUEMENT en mémoire (jamais cookie, jamais
   localStorage/sessionStorage) : il repart dans `X-MCC-Session` à chaque appel
   /api/*. Le moteur exige les deux (cookie + secret). Puis on efface le
   fragment : l'URL redevient celle qu'on peut copier sans rien livrer. */
let sessionSecret = '';
let sessionLost = false;
let _updateSessionClosed = false;
const SESSION_LOST_MSG = 'session expirée — rouvre le lien de démarrage (MCP Control)';
async function exchangeSession() {
  let token = '';
  try {
    const m = /[#&]t=([^&\s]+)/.exec(window.location.hash || '');
    if (m) token = decodeURIComponent(m[1]);
  } catch (e) { token = ''; }
  if (!token) return false;
  let ok = false;
  try {
    const r = await _mccFetch('/api/session', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({token: token})
    });
    const j = await r.json().catch(() => null);
    if (r.ok && j && typeof j.secret === 'string' && j.secret) {
      sessionSecret = j.secret; // mémoire seulement : jamais cookie ni stockage
      ok = true;
    }
  } catch (e) { ok = false; }
  // fragment effacé DANS TOUS LES CAS : le jeton est à usage unique, le garder
  // dans la barre d'adresse ne ferait que l'afficher.
  try { history.replaceState(null, '', window.location.pathname + window.location.search); } catch (e) {}
  return ok;
}
/* Une seule tentative par chargement ; toutes les requêtes /api/* attendent
   son issue (le cookie doit exister avant la première lecture d'état). */
const sessionReady = exchangeSession();

/* MC-C01 : récupération transparente de session appelée par le wrapper Desktop
   (Tauri) au retour du tray, après mise en veille prolongée (> 12h/24h) ou F5.
   Ne fait rien si la session courante est déjà active. */
window.__mccRecoverSession = async function(token) {
  if (!token || (!sessionLost && sessionSecret)) return false;
  try {
    const r = await _mccFetch('/api/session', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify({token: String(token)})
    });
    const j = await r.json().catch(() => null);
    if (r.ok && j && typeof j.secret === 'string' && j.secret) {
      sessionSecret = j.secret;
      sessionLost = false;
      _updateSessionClosed = false;
      setDead(false);
      await load();
      return true;
    }
  } catch (e) {}
  return false;
};

window.fetch = async (url, opts = {}) => {
  const isApi = typeof url === 'string' && url.startsWith('/api/');
  // Hors /api/* : requête transmise TELLE QUELLE. Indispensable pour l'IPC
  // Tauri (POST http://ipc.localhost/…) dont les en-têtes arrivent en objet
  // Headers — les recopier via Object.assign les perdait (Tauri-Invoke-Key
  // absent → 500, plus aucun drag / réduire / agrandir / fermer).
  if (!isApi) return _mccFetch(url, opts);
  await sessionReady;
  // Session perdue : plus AUCUNE requête /api/* ne part (U-9) — sans cela le
  // polleur rejouerait un 401 à chaque cycle. Réponse synthétique locale.
  if (sessionLost) {
    return new Response(JSON.stringify({ok: false, error: SESSION_LOST_MSG}),
      {status: 401, headers: {'Content-Type': 'application/json'}});
  }
  // new Headers() accepte objet simple, tableau ou Headers sans rien perdre.
  const h = new Headers(opts.headers || {});
  const code = accessCode();
  if (code) h.set('X-MCC-Access', code);
  if (sessionSecret) h.set('X-MCC-Session', sessionSecret);
  const r = await _mccFetch(url, Object.assign({}, opts, {headers: h}));
  if (r.status === 401) onUnauthorized();
  return r;
};
/* 401 de session (U-9) : on n'ouvre plus le dialogue de l'ancien code d'accès.
   Un code hérité réellement stocké (bind LAN, lecture seule) garde son dialogue ;
   sinon on annonce « session expirée » UNE fois et on cesse d'appeler /api/*. */
function onUnauthorized() {
  if (_updateRestarting) {
    _updateRestarting = false;
    _updateRestartSawDown = false;
    _updatingNow = false;
    sessionLost = true;
    _updateSessionClosed = true;
    setDead(false);
    renderUpdateProgress({
      active: false,
      phase: 'done',
      version: _updateTargetVer || null,
      bytes_downloaded: 0,
      bytes_total: null,
      percent: 100,
      message: 'Mise \u00e0 jour install\u00e9e \u2014 nouvelle version d\u00e9marr\u00e9e. Recharge ou ouvre la nouvelle fen\u00eatre.',
      error: null,
    });
    toast('Nouvelle version d\u00e9marr\u00e9e', 'ok', 6000);
    return;
  }
  if (accessCode()) { showLogin(); return; }
  if (sessionLost) return;
  sessionLost = true;
  toast(SESSION_LOST_MSG, 'err', 8000);
  setDead(true, SESSION_LOST_MSG);
}
function showLogin() {
  const d = document.getElementById('loginDlg');
  if (d && !d.open) {
    rememberFocus();
    d.showModal();
    const inp = document.getElementById('loginCode');
    if (inp) inp.focus();
  }
}
async function loginSubmit(ev) {
  if (ev && ev.preventDefault) ev.preventDefault();
  const codeEl = document.getElementById('loginCode');
  const code = codeEl ? codeEl.value.trim() : '';
  const msg = document.getElementById('loginMsg');
  try {
    // fetch brut (pas l'intercepteur) : pas de boucle 401 pendant la vérif
    const r = await _mccFetch('/api/state', {headers: code ? {'X-MCC-Access': code} : {}});
    if (r.ok) {
      try { localStorage.setItem('mcc.access', code); } catch (e) {}
      if (msg) msg.textContent = '';
      if (codeEl) codeEl.value = '';
      const d = document.getElementById('loginDlg');
      if (d && d.open) d.close();
      restoreFocus();
      updateLogoutBtn();
      load();
    } else if (msg) {
      msg.textContent = '\u2717 code refus\u00e9 — voir settings.api_access du config.json du serveur';
    }
  } catch (e) {
    if (msg) msg.textContent = '\u2717 erreur r\u00e9seau lors de la v\u00e9rification';
  }
  return false;
}

// « Oublier le code » : rien ne doit rester dans le navigateur après un poste
// partagé ou une rotation de code côté serveur.
function forgetCode() {
  try { localStorage.removeItem('mcc.access'); } catch (e) {}
  const msg = document.getElementById('loginMsg');
  if (msg) msg.textContent = '\u2713 code effac\u00e9 de ce navigateur';
  updateLogoutBtn();
}

// Le bouton de déconnexion n'apparaît que si un code est stocké.
function updateLogoutBtn() {
  const b = document.getElementById('tbLogout');
  if (b) b.hidden = !accessCode();
}
function logout() {
  forgetCode();
  const d = document.getElementById('loginDlg');
  if (d) {
    if (!d.open) { rememberFocus(); d.showModal(); }
    const inp = document.getElementById('loginCode');
    if (inp) inp.focus();
  }
  load();
}

let _sshDiscoverIv = null;
(function () {
  const cd = document.getElementById('confirmDlg');
  if (cd) {
    cd.addEventListener('cancel', e => { e.preventDefault(); confirmDlgCancel(); });
    cd.addEventListener('click', e => { if (e.target === cd) confirmDlgCancel(); });
    cd.addEventListener('close', () => { if (_confirmResolve) finishConfirmDlg(false); });
    cd.addEventListener('keydown', e => {
      if (e.key === 'Enter' && e.target && e.target.tagName === 'INPUT' && e.target.closest('#confirmDlgInputs')) {
        e.preventDefault();
        confirmDlgOk();
      }
    });
  }
  const ld = document.getElementById('loginDlg');
  if (ld) {
    ld.addEventListener('cancel', e => {
      e.preventDefault();
      const inp = document.getElementById('loginCode');
      if (inp) inp.focus();
    });
  }
  const wireBasicDlg = id => {
    const d = document.getElementById(id);
    if (!d) return;
    d.addEventListener('cancel', e => { e.preventDefault(); d.close(); restoreFocus(); });
    d.addEventListener('click', e => { if (e.target === d) { d.close(); restoreFocus(); } });
    if (id === 'machineDlg') {
      d.addEventListener('close', () => {
        if (_sshDiscoverIv) { clearInterval(_sshDiscoverIv); _sshDiscoverIv = null; }
      });
    }
  };
  wireBasicDlg('dlg');
  wireBasicDlg('storeDlg');
  wireBasicDlg('machineDlg');
})();

let state = null;
let selectedId = null;
let detailCache = null;
let detailTab = 'outils';
// Échappement HTML de TOUT texte venu d'un client. Les caractères de
// contrôle (C0 et DEL) sont retirés : un libellé hostile ne doit ni sortir de
// son contexte ni injecter de séquence de terminal (lot M3).
const esc = s => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c])).replace(/[\u0000-\u001f\u007f]/g, '');
// Variante multi-lignes pour <pre> et <textarea> : préserve \n et \t tout en
// neutralisant tout caractère de contrôle résiduel et toute balise HTML.
const escPre = s => String(s ?? '').replace(/\r\n?/g, '\n').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c])).replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, '');
let overviewFilter = null; // filtre de tuile d'accueil : 'up' = ne montrer que les UP

/* ===== Icônes =============================================================
   Un seul jeu SVG inline (trait 1.5, couleur courante, zéro emoji) : rendu
   identique quelle que soit la version d'Windows et alignement typographique
   propre dans une interface monochrome. Chaque icône est inlinée par ic() —
   pas de référence à un symbole (guillemet-double suivi de dièse interdit
   dans la raw string Rust qui embarque cette page), attributs en simples
   quotes comme le reste des SVG de la page. */
const ICONS = {
  search: "<circle cx='11' cy='11' r='7'/><path d='m21 21-4.3-4.3'/>",
  globe: "<circle cx='12' cy='12' r='8.5'/><path d='M3.5 12h17M12 3.5c2.4 2.4 3.8 5.3 3.8 8.5s-1.4 6.1-3.8 8.5c-2.4-2.4-3.8-5.3-3.8-8.5s1.4-6.1 3.8-8.5Z'/>",
  monitor: "<rect x='2.5' y='4' width='19' height='13' rx='2'/><path d='M8 21h8M12 17v4'/>",
  cart: "<circle cx='9' cy='20' r='1.4'/><circle cx='17' cy='20' r='1.4'/><path d='M2.5 3.5h2.2l2.5 11.5h10.3l2-8H6'/>",
  gear: "<circle cx='12' cy='12' r='3.1'/><path d='M12 2.8v2.6M12 18.6v2.6M2.8 12h2.6M18.6 12h2.6M5.5 5.5l1.9 1.9M16.6 16.6l1.9 1.9M18.5 5.5l-1.9 1.9M7.4 16.6l-1.9 1.9'/>",
  refresh: "<path d='M21 12a9 9 0 1 1-2.6-6.4L21 8'/><path d='M21 3v5h-5'/>",
  power: "<path d='M18.4 6.6a9 9 0 1 1-12.8 0'/><path d='M12 2.5V12'/>",
  lock: "<rect x='4.5' y='10.5' width='15' height='10' rx='2'/><path d='M8 10.5V7a4 4 0 0 1 8 0v3.5'/>",
  zap: "<path d='M13 2.5 4.5 13.5H11l-1 8 8.5-11H12l1-8Z'/>",
  star: "<path d='m12 3.6 2.5 5.1 5.7.8-4.1 4 1 5.6L12 16.4l-5.1 2.7 1-5.6-4.1-4 5.7-.8Z'/>",
  eye: "<path d='M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z'/><circle cx='12' cy='12' r='3'/>",
  eyeOff: "<path d='M4 4l16 16'/><path d='M10.3 5.7a9.8 9.8 0 0 1 1.7-.2c6 0 9.5 6.5 9.5 6.5a17.6 17.6 0 0 1-3 3.8M6.7 6.7C4.1 8.4 2.5 12 2.5 12S6 18.5 12 18.5c1.3 0 2.5-.3 3.6-.8'/><path d='M9.9 9.9a3 3 0 0 0 4.2 4.2'/>",
  play: "<path d='M7 4.5 19 12 7 19.5Z'/>",
  stop: "<rect x='6' y='6' width='12' height='12' rx='1.5'/>",
  pause: "<path d='M9 5v14M15 5v14'/>",
  trash: "<path d='M3.5 6h17M8.5 6V4.5A1.5 1.5 0 0 1 10 3h4a1.5 1.5 0 0 1 1.5 1.5V6M19 6l-.8 13a2 2 0 0 1-2 1.9H7.8a2 2 0 0 1-2-1.9L5 6'/><path d='M10 10.5v6M14 10.5v6'/>",
  copy: "<rect x='9' y='9' width='12' height='12' rx='2'/><path d='M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1'/>",
  tag: "<path d='m3 11 8-8h9v9l-8.5 8.5a2 2 0 0 1-2.8 0L3 13.8a2 2 0 0 1 0-2.8Z'/><circle cx='16.5' cy='7.5' r='1.3'/>",
  link: "<path d='M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7'/><path d='M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7'/>",
  terminal: "<path d='m4 17 6-5-6-5'/><path d='M12 19h8'/>",
  folder: "<path d='M3.5 7A1.5 1.5 0 0 1 5 5.5h4.2l1.8 2h9A1.5 1.5 0 0 1 21.5 9v8a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 17Z'/>",
  external: "<path d='M14 4h6v6M20 4l-9 9'/><path d='M19 14v5a1.5 1.5 0 0 1-1.5 1.5h-12A1.5 1.5 0 0 1 4 19V7.5A1.5 1.5 0 0 1 5.5 6H10'/>",
  activity: "<path d='M22 12h-4l-3 8L9 4l-3 8H2'/>",
  check: "<path d='m4.5 12.5 5 5L19.5 7'/>",
  x: "<path d='m6 6 12 12M18 6 6 18'/>",
  bell: "<path d='M6 9a6 6 0 0 1 12 0c0 5 2 6.5 2 6.5H4S6 14 6 9Z'/><path d='M10 19.5a2.2 2.2 0 0 0 4 0'/>",
  upload: "<path d='M12 15.5V4m0 0L7.5 8.5M12 4l4.5 4.5'/><path d='M4 16.5V19a1.5 1.5 0 0 0 1.5 1.5h13A1.5 1.5 0 0 0 20 19v-2.5'/>",
  download: "<path d='M12 4v11.5m0 0L7.5 11M12 15.5l4.5-4.5'/><path d='M4 16.5V19a1.5 1.5 0 0 0 1.5 1.5h13A1.5 1.5 0 0 0 20 19v-2.5'/>",
  warn: "<path d='M12 3.5 2.5 20h19L12 3.5Z'/><path d='M12 10v4.5M12 17.1v.1'/>",
  shield: "<path d='M12 3 4.5 6v5.5c0 4.4 3.1 8.3 7.5 9.5 4.4-1.2 7.5-5.1 7.5-9.5V6L12 3Z'/><path d='m8.8 11.8 2.2 2.2 4.2-4.4'/>",
  save: "<path d='M5 3.5h11L20.5 8v11a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 19V5A1.5 1.5 0 0 1 5 3.5Z'/><path d='M8 3.5V8h7V3.5M7.5 20.5v-6h9v6'/>",
};
const ic = n => `<svg class='ic' viewBox='0 0 24 24' aria-hidden='true'>${ICONS[n] || ''}</svg>`;

/* ===== Santé moteur : bandeau « injoignable » + compteurs grisés =====
   Chaque échec du poll compte ; dès 2 échecs consécutifs le bandeau apparaît
   et les compteurs se grisent — des chiffres périmés ne passent plus pour à
   jour sans signal (audit 04 #3 : catch vide). Premier chargement jamais
   réussi : même bandeau, libellé « nouvel essai » (audit 04 #2). Le retour
   est automatique : le premier état reçu lève bandeau et grisage. */
let pollFails = 0;
const DEAD_BOOT_MSG = 'moteur injoignable, nouvel essai…';
const DEAD_MSG = 'moteur injoignable';
let _lastSelectedSrvSig = '';
let _lastAccessRevision = null;
let _stateEtag = null;
let _detailEtag = null;
let _detailEtagId = null;
let _sigServers = '';
let _sigPeers = '';
let _sigAnn = '';
let _sigLog = '';
let _sigOverview = '';
let _sigDetail = '';
let _sigAccess = '';
const setHidden = (el, h) => { if (el && el.hidden !== !!h) el.hidden = !!h; };
const setText = (el, txt) => { const s = String(txt ?? ''); if (el && el.textContent !== s) el.textContent = s; };

function setDead(on, msg) {
  const b = document.getElementById('deadBanner');
  if (b) {
    setHidden(b, !on);
    if (on) setText(b, msg);
  }
  if (document.body.classList.contains('dead') !== !!on) {
    document.body.classList.toggle('dead', on);
  }
  // boot jamais réussi : la zone principale est vide — y porter le message,
  // render() l'écrasera à la reprise.
  const c = document.getElementById('content');
  if (on && !state && c && !c.querySelector('.dash-head,.d-head,.engine-wait')) {
    c.innerHTML = '<h1 class="sr-only">MCP Control</h1><div class="engine-wait">' + esc(msg) + '</div>';
  }
}
let _detailReqSeq = 0;
async function load() {
  if (document.hidden) return; // fenêtre masquée (tray) : polling suspendu — CPU/mémoire au repos
  try {
    const stOpts = (state && _stateEtag) ? {headers: {'If-None-Match': _stateEtag}} : {};
    const r = await fetch('/api/state', stOpts);
    if (r.status === 304) {
      pollFails = 0;
      setDead(false);
      if (_updateRestarting && _updateRestartSawDown) {
        _updateRestarting = false;
        _updateRestartSawDown = false;
        _updatingNow = false;
        toast('Nouvelle version d\u00e9marr\u00e9e \u2014 rechargement\u2026', 'ok');
        setTimeout(() => location.reload(), 400);
        return;
      }
      armPolling();
    } else {
      // Un 401 (session expirée) n'est pas un état : ne jamais le rendre.
      if (!r.ok) throw new Error('HTTP ' + r.status);
      _stateEtag = r.headers.get('ETag') || null;
      state = await r.json();
      pollFails = 0;
      setDead(false);
      if (selectedId && !state.servers.find(s => s.id === selectedId)) {
        selectedId = null; detailCache = null; _lastSelectedSrvSig = ''; _detailEtag = null; _detailEtagId = null; _sigOverview = '';
      }
      render();
      if (!_storeVersionsFetched) {
        _storeVersionsFetched = true;
        const hasStoreSrv = (state.servers || []).some(s => s.id && s.id.startsWith('store-'));
        const verUrl = hasStoreSrv ? '/api/store-versions?installed=1' : '/api/store-versions';
        fetch(verUrl).then(rv => rv.ok ? rv.json() : null).then(v => {
          if (!v) return;
          storeVersions = Object.assign(storeVersions || {}, v);
          renderServers();
          const sd = document.getElementById('storeDlg');
          if (sd && sd.open) renderStoreItems();
        }).catch(() => {});
      }
      if (_updateRestarting && (_updateRestartSawDown || (state && state.update_status && state.update_status.phase === 'idle'))) {
        _updateRestarting = false;
        _updateRestartSawDown = false;
        _updatingNow = false;
        toast('Nouvelle version d\u00e9marr\u00e9e \u2014 rechargement\u2026', 'ok');
        setTimeout(() => location.reload(), 400);
        return;
      }
      const rev = state && state.access ? state.access.revision : null;
      if (rev !== _lastAccessRevision) {
        _lastAccessRevision = rev;
        const ad = document.getElementById('accessDlg');
        if (ad && ad.open) refreshAccess();
      }
    }
    if (selectedId && state) {
      const targetId = selectedId;
      const needFreshDetail = !detailCache || detailCache.id !== targetId || _detailEtagId !== targetId;
      if (needFreshDetail) {
        _detailEtag = null;
        _detailEtagId = targetId;
      }
      const srvSummary = state.servers.find(s => s.id === targetId);
      const srvSig = JSON.stringify(srvSummary || null);
      if (needFreshDetail || srvSig !== _lastSelectedSrvSig) {
        _lastSelectedSrvSig = srvSig;
        const reqSeq = ++_detailReqSeq;
        const dOpts = (!needFreshDetail && _detailEtag) ? {headers: {'If-None-Match': _detailEtag}} : {};
        const r2 = await fetch('/api/detail?id=' + encodeURIComponent(targetId), dOpts);
        if (reqSeq !== _detailReqSeq || selectedId !== targetId) return;
        if (r2.status === 304) return;
        if (r2.ok) {
          _detailEtag = r2.headers.get('ETag') || null;
          detailCache = await r2.json();
          if (reqSeq === _detailReqSeq && selectedId === targetId) renderDetail();
        }
      }
    }
  } catch(e) {
    if (sessionLost) {
      if (!_updateSessionClosed) setDead(true, SESSION_LOST_MSG);
      return;
    }
    if (_updateRestarting) {
      _updateRestartSawDown = true;
      setDead(false);
      renderUpdateProgress({
        active: true,
        phase: 'restarting',
        version: _updateTargetVer || null,
        bytes_downloaded: 0,
        bytes_total: null,
        percent: 100,
        message: 'Red\u00e9marrage vers la nouvelle version \u2014 reconnexion en cours\u2026',
        error: null,
      });
      scheduleNext(1000);
      return;
    }
    if (document.getElementById('loginDlg')?.open) return;
    pollFails++;
    if (state === null) setDead(true, DEAD_BOOT_MSG);
    else if (pollFails >= 2) setDead(true, DEAD_MSG);
    scheduleNext(POLL_RETRY_MS);
  }
}

function render() {
  if (!state) return;
  const s = state.stats;
  const nw = document.getElementById('netWarn');
  if (nw) {
    const off = state.lan_ok === false;
    setHidden(nw, !off);
    if (off && !nw.innerHTML) nw.innerHTML = ic('warn') + '<span>R&eacute;seau local injoignable</span>';
  }
  setText(document.getElementById('stUp'), s.up);
  setText(document.getElementById('stTotal'), s.total);
  setText(document.getElementById('stTools'), s.tools);
  armPolling();
  const au = document.getElementById('stAuto');
  if (au) {
    const autoKey = state.autostart ? '1' : '0';
    if (au.dataset.stateKey !== autoKey) {
      au.dataset.stateKey = autoKey;
      au.innerHTML = ic('power') + '<span>Auto-boot ' + (state.autostart ? 'ON' : 'OFF') + '</span>';
      au.setAttribute('aria-checked', state.autostart ? 'true' : 'false');
      au.classList.toggle('auto-on', !!state.autostart);
      au.classList.toggle('auto-off', !state.autostart);
    }
  }
  const upd = document.getElementById('stUpdate');
  const uSt = state.update_status || null;
  if (upd) {
    if (uSt && uSt.active) {
      setHidden(upd, false);
      const pct = (uSt.phase === 'downloading' && typeof uSt.percent === 'number')
        ? (' ' + uSt.percent + '%') : '';
      const phaseTxt = uSt.phase === 'restarting' ? 'Red\u00e9marrage\u2026'
        : uSt.phase === 'verifying_sha256' ? 'V\u00e9rif SHA-256\u2026'
        : uSt.phase === 'installing' ? 'Installation\u2026'
        : 'Mise \u00e0 jour' + pct + '\u2026';
      delete upd.dataset.ver;
      upd.innerHTML = ic('download') + '<span class="tb-lbl">' + esc(phaseTxt) + '</span>';
      upd.title = updatePhaseLabel(uSt) || 'Mise \u00e0 jour en cours';
    } else if (state.update_available && state.latest_version) {
      setHidden(upd, false);
      if (upd.dataset.ver !== String(state.latest_version)) {
        upd.dataset.ver = String(state.latest_version);
        upd.innerHTML = ic('download') + '<span class="tb-lbl">Mise &agrave; jour v' + esc(String(state.latest_version)) + '</span>';
        upd.title = 'Mettre à jour vers v' + state.latest_version + ' (installée : v' + MCC_VERSION
          + ') — clic : téléchargement et installation directe';
      }
    } else setHidden(upd, true);
  }
  renderUpdateProgress(uSt);
  renderServers();
  renderPeers();
  syncAnnouncements();
  renderLog();
  accessBadge();
  if (typeof updateSwarmBadge === 'function') updateSwarmBadge();
  if (typeof updateSwarmTopbarActive === 'function') updateSwarmTopbarActive();
  if (!selectedId && activeView !== 'swarm') renderOverview();
}

/* ===== Annonces LiteMCP détectées, non suivies (lot S4 pt 2) =====
   Lecture seule : jamais d'adoption silencieuse. L'opérateur clique « Suivre »
   pour ajouter explicitement le serveur HTTP à sa configuration. */
let unfollowedAnnouncements = [];
let lastAnnCount = -1;
async function syncAnnouncements() {
  const count = (state && Number.isFinite(state.announcements_count)) ? state.announcements_count : 0;
  if (count === 0) {
    lastAnnCount = 0;
    if (unfollowedAnnouncements.length) {
      unfollowedAnnouncements = [];
      renderAnnouncements();
      if (!selectedId) renderOverview();
    } else {
      renderAnnouncements();
    }
    return;
  }
  if (count === lastAnnCount) {
    renderAnnouncements();
    return;
  }
  lastAnnCount = count;
  const j = await postAction({action: 'litemcp_announcements'}, {quiet: true});
  if (j && j.ok && Array.isArray(j.announcements)) {
    unfollowedAnnouncements = j.announcements.filter(a => a && a.followed === false);
  } else {
    unfollowedAnnouncements = [];
  }
  renderAnnouncements();
  if (!selectedId) renderOverview();
}
function renderAnnouncements() {
  const sec = document.getElementById('annSec');
  const box = document.getElementById('annList');
  if (!sec || !box) return;
  const sig = JSON.stringify(unfollowedAnnouncements);
  if (sig === _sigAnn) return;
  _sigAnn = sig;
  if (!unfollowedAnnouncements.length) {
    setHidden(sec, true);
    box.innerHTML = '';
    return;
  }
  setHidden(sec, false);
  box.innerHTML = unfollowedAnnouncements.map(a => {
    const portTxt = a.port ? `:${esc(a.port)}` : 'port inconnu';
    const verTxt = a.version ? ` &middot; v${esc(a.version)}` : '';
    const btn = a.port
      ? `<button class="primary" data-act="follow-ann" data-name="${esc(a.name)}" data-port="${esc(a.port)}" title="Ajouter explicitement ce serveur LiteMCP &agrave; la liste suivie">${ic('check')} Suivre</button>`
      : '';
    return `<div class="ann-row">
      <div class="l1"><span class="dot PORT-ONLY"></span><span class="nm">${esc(a.name)}</span><span class="tag litemcp">LiteMCP</span></div>
      <div class="l2">127.0.0.1${portTxt}${verTxt} &mdash; d&eacute;tect&eacute;, non suivi</div>
      ${btn}
    </div>`;
  }).join('');
}
async function followAnnouncement(name, port) {
  const p = +port;
  if (!p) { toast('Port d\u2019annonce manquant', 'err'); return; }
  const j = await postAction({action: 'add', transport: 'http', label: name || ('litemcp-' + p), port: p, path: '/mcp'});
  if (j && j.ok) {
    toast('Serveur \u00ab ' + (name || p) + ' \u00bb ajout\u00e9 \u00e0 la supervision', 'ok');
    lastAnnCount = -1;
    await act('refresh', j.id);
  }
}

/* ===== Explorateur ===== */
/* Clé de groupement : 'local' (loopback de cette machine) ou host/IP de la
   machine distante (LAN/VPS). L'explorateur affiche les locaux à plat puis
   UN GROUPE REPLIABLE PAR MACHINE distante. */
/* Hôtes loopback EXPLICITES = CETTE machine : sans cela un serveur déclaré
   sur 127.0.0.1 / ::1 / localhost atterrissait sous « Machines distantes »
   et perdait son formulaire de commande de lancement (audit 04 #6). */
const isLoopbackHost = h => ['127.0.0.1', '::1', 'localhost'].includes(String(h || '').toLowerCase());
function hostOf(s) {
  if (s.transport !== 'http') return 'local';
  const h = s.transport_info.host;
  return (!h || isLoopbackHost(h)) ? 'local' : h;
}
/* Helper global : serveur par id (applySort a son propre byId local, ombre légale). */
function byId(id) {
  return state && state.servers ? state.servers.find(s => s.id === id) : null;
}
function groupKeys() {
  // hosts des serveurs distants (ordre d'affichage) + machines enregistrées
  // (ssh_hosts) même sans serveur importé — le groupe vide montre où ranger.
  const keys = [];
  state.servers.forEach(s => { const k = hostOf(s); if (k !== 'local' && !keys.includes(k)) keys.push(k); });
  (state.settings.ssh_hosts || []).forEach(h => { if (!keys.includes(h.ip)) keys.push(h.ip); });
  return keys;
}
function hybridUrl(s) {
  if (!s || !s.hybrid_path) return '';
  return window.location.origin + s.hybrid_path;
}
function logoContrastCls(inner) {
  const matches = String(inner || '').match(/#([0-9a-fA-F]{3}|[0-9a-fA-F]{6})\b/g);
  if (!matches || !matches.length) return '';
  let maxLum = 0, minLum = 1, maxChroma = 0;
  for (const hex of matches) {
    let h = hex.slice(1);
    if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
    const r = parseInt(h.slice(0, 2), 16) / 255;
    const g = parseInt(h.slice(2, 4), 16) / 255;
    const b = parseInt(h.slice(4, 6), 16) / 255;
    const lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    const chroma = Math.max(r, g, b) - Math.min(r, g, b);
    if (lum > maxLum) maxLum = lum;
    if (lum < minLum) minLum = lum;
    if (chroma > maxChroma) maxChroma = chroma;
  }
  if (maxChroma <= 0.15 && minLum >= 0.78) return ' logo-on-dark';
  if (maxChroma <= 0.15 && maxLum <= 0.26) return ' logo-on-light';
  return '';
}
function swarmBadge(s) {
  if (!s || !s.swarm_kind) return '';
  if (s.swarm_kind === 'resident') {
    const ag = s.swarm_agent ? ' : ' + s.swarm_agent : '';
    return `<span class="tag swarm-res" title="Agent MCP r&eacute;sident persistant Swarm-control${esc(ag)}">R&Eacute;SIDENT</span>`;
  }
  return '<span class="tag swarm" title="Instance d&#39;orchestration Swarm-control (LiteMCP)">SWARM</span>';
}
function srvStoreLogo(s, lg) {
  if (!s) return '';
  if (s.swarm_kind) {
    const isRes = s.swarm_kind === 'resident';
    const tip = isRes
      ? ('Swarm-control — Agent MCP r\u00e9sident' + (s.swarm_agent ? ' (' + s.swarm_agent + ')' : ''))
      : 'Swarm-control — Instance d\u2019orchestration MCP';
    const src = swarmLogoUrl(s.swarm_agent || s.label, s.swarm_kind, s.swarm_class);
    return `<span class="srv-logo swarm-logo${isRes ? ' swarm-res' : ''}${lg ? ' lg' : ''}" title="${esc(tip)}"><img class="swarm-img" src="${esc(src)}" alt="" aria-hidden="true"></span>`;
  }
  if (!s.id || !s.id.startsWith('store-')) return '';
  let inner = PKG_GLYPH;
  if (storeLogos) {
    const raw = s.id.slice(6);
    const stripped = raw.replace(/^(at|gl|gh)-/, '');
    const noNum = raw.replace(/-\d+$/, '');
    const found = storeLogos[raw] || storeLogos[stripped] || storeLogos[noNum];
    if (found) {
      inner = found;
    } else {
      const sc = s.start_cmd || s.transport_info || {};
      const cmd = ((sc.program || '') + ' ' + (sc.args || []).join(' ')).toLowerCase();
      for (const [needle, lid] of PKG_LOGOS) {
        if (storeLogos[lid] && cmd.includes(needle)) { inner = storeLogos[lid]; break; }
      }
    }
  }
  return `<span class="srv-logo${lg ? ' lg' : ''}${logoContrastCls(inner)}" title="Serveur install&eacute; depuis la Boutique MCP Control"><svg viewBox="0 0 24 24" aria-hidden="true">${inner}</svg></span>`;
}
function loadStoreLogos() {
  if (storeLogos) return;
  fetch('/api/store-logos').then(r => r.ok ? r.json() : null).then(j => {
    if (!j) return;
    storeLogos = j;
    _sigServers = '';
    _sigDetail = '';
    if (state) {
      renderServers();
      if (selectedId && detailCache) renderDetail();
    }
  }).catch(() => {});
}

function swarmHiveRow() {
  const st = (state && state.swarm_stats) || {instances: 0, residents: 0, up: 0};
  const nInst = st.instances || 0;
  const nRes = st.residents || 0;
  const up = st.up || 0;
  const stCls = up > 0 ? 'UP' : ((nInst > 0 || nRes > 0) ? 'DOWN' : 'PORT-ONLY');
  const isSel = activeView === 'swarm';
  const summary = nRes > 0
    ? `${nRes} agent${nRes > 1 ? 's' : ''} r\u00e9sid.`
    : (nInst > 0 ? `${nInst} orchestrateur${nInst > 1 ? 's' : ''}` : 'orchestration \u00b7 A2A');
  return `<div class="srow srow-hive ${isSel ? 'sel' : ''}" draggable="false"
    role="listitem" tabindex="0"${isSel ? ' aria-current="true"' : ''}
    aria-label="Essaim Swarm Control — orchestration, agents persistants et interactions A2A"
    data-act="swarm-open">
    <div class="l1">
      <span class="dot ${esc(stCls)}"></span>
      <span class="srv-logo swarm-logo" title="Essaim Swarm-control — Orchestration &amp; A2A"><img class="swarm-img" src="${esc(swarmLogoUrl('logo', 'hive'))}" alt="" aria-hidden="true"></span>
      <span class="nm" title="Essaim Swarm — Interactions entre agents, orchestration et A2A">Essaim Swarm</span>
      <div class="l1-right">
        <span class="cnt" title="${esc(String(nRes))} agent(s) r\u00e9sident(s)">${esc(String(nRes))}</span>
      </div>
    </div>
    <div class="l2">
      <div class="srow-meta">
        <span class="tag swarm">ESSAIM</span>
        <span class="tag sw-a2a">A2A</span>
        <span class="srow-pill">${esc(summary)}</span>
      </div>
    </div>
  </div>`;
}
function srvRow(s) {
  const endpoint = s.transport === 'http'
    ? (s.transport_info.host && !isLoopbackHost(s.transport_info.host)
        ? `${esc(s.transport_info.host)}:${esc(s.transport_info.port)}`
        : `:${esc(s.transport_info.port)}`)
    : esc(s.transport_info.program || 'stdio');
  const offCount = (s.tools_disabled || []).length;
  const offPill = offCount
    ? `<span class="cnt-off" title="${esc(offCount)} outil(s) d&eacute;sactiv&eacute;(s)">&minus;${esc(offCount)}</span>`
    : '';
  const latPill = (s.state === 'UP' && s.latency_ms != null)
    ? `<span class="srow-lat">${esc(String(s.latency_ms))}ms</span>`
    : '';
  const storeId = (s.id && s.id.startsWith('store-')) ? s.id.slice(6) : '';
  const remoteStoreVer = (storeId && storeVersions && storeVersions[storeId]) || '';
  const srvVer = s.version || '';
  const verPill = (srvVer && remoteStoreVer && srvVer.replace(/^v/i, '') !== remoteStoreVer.replace(/^v/i, ''))
    ? `<button type="button" class="srow-ver srow-ver-upd srow-ver-btn" data-act="open-store-update" data-store-id="${esc(storeId)}" title="Mise \u00e0 jour disponible (${esc(fmtStoreVer(srvVer))} \u2192 ${esc(fmtStoreVer(remoteStoreVer))}) \u2014 ouvrir la boutique">${ic('download')} ${esc(fmtStoreVer(srvVer))} \u2192 ${esc(fmtStoreVer(remoteStoreVer))}</button>`
    : srvVer
      ? `<span class="srow-ver" title="Version du serveur MCP">${esc(fmtStoreVer(srvVer))}</span>`
      : '';
  const stPill = s.state !== 'UP'
    ? `<span class="st ${esc(s.state)}">${esc(s.state)}</span>`
    : '';
  const toolsTip = `${s.tools_count} outil${s.tools_count > 1 ? 's' : ''}`
    + (offCount ? ` (${offCount} coup\u00e9s)` : '');
  const isSel = selectedId === s.id;
  const isMulti = selectedServers.size > 1 && selectedServers.has(s.id);
  /* Ligne 1 : état + logo boutique éventuel + nom + compteur d'outils + switch ON/OFF.
     Ligne 2 : métadonnées (état si hors-ligne, port/programme, latence, badges)
     à gauche, actions rapides (favori, autoboot, play/stop) séparées à droite. */
  return `<div class="srow ${isSel?'sel':''} ${isMulti?'multi-sel':''} ${s.enabled?'':'off'}" draggable="true"
    role="listitem" tabindex="0"${isSel ? ' aria-current="true"' : ''}${isMulti ? ' aria-selected="true"' : ''} aria-label="Serveur ${esc(s.label)} (${esc(s.state)})"
    data-act="select" data-id="${esc(s.id)}">
    <div class="l1">
      <span class="dot ${esc(s.state)}"></span>${srvStoreLogo(s, false)}
      <span class="nm" title="${esc(s.label)}">${esc(s.label)}</span>
      <div class="l1-right">
        <span class="cnt" title="${esc(toolsTip)}">${esc(s.tools_count)}</span>${offPill}${rowToggle(s)}
      </div>
    </div>
    <div class="l2">
      <div class="srow-meta">
        ${stPill}<span class="srow-pill" title="${esc(endpoint)}">${endpoint}</span>${latPill}${verPill}${langChip(s)}${dockerBadge(s, false)}${swarmBadge(s)}${frameworkBadge(s)}${lockBadge(s)}${s.transport==='stdio'?'<span class="tag hybrid" title="Pont hybride natif : Streamable HTTP (/mcp/s/...) + auth Ed25519 LiteMCP">HTTP+AUTH</span>':''}
      </div>
      <div class="srow-acts">${rowFav(s)}${rowAuto(s)}${rowCtl(s)}</div>
    </div>
  </div>`;
}
/* Replis des groupes mémorisés (localStorage) : 'local' + une clé par machine. */
function groupClosed(key) {
  try { return localStorage.getItem('mcc.group.' + key) === '1'; } catch (e) { return false; }
}
function toggleGroup(key) {
  try { localStorage.setItem('mcc.group.' + key, groupClosed(key) ? '0' : '1'); } catch (e) {}
  renderServers();
}
function groupHeadHtml(key, label, up, total, machine, bodyId) {
  const closed = groupClosed(key);
  const tip = machine
    ? `Machine distante ${esc(label)} &mdash; cliquer pour replier/d&eacute;plier`
    : 'Serveurs MCP locaux &agrave; cette machine (loopback) &mdash; cliquer pour replier/d&eacute;plier';
  const act = machine
    ? `<button type="button" class="gact" aria-label="Oublier la machine ${esc(label)}" title="Oublier cette machine (les serveurs d&eacute;j&agrave; import&eacute;s restent)" data-act="ssh-forget" data-id="${esc(label)}">${ic('trash')}</button>` : '';
  const ctrlAttr = bodyId ? ` aria-controls="${esc(bodyId)}"` : '';
  return `<div class="grphead ${closed?'closed':''}" data-key="${esc(key)}"><button type="button" class="grp-toggle" data-act="group-toggle" data-key="${esc(key)}" aria-expanded="${closed ? 'false' : 'true'}"${ctrlAttr} title="${tip}"><span class="gchev" aria-hidden="true">&#9662;</span><span class="gname">${esc(label)}</span><span class="gcount">${esc(up)}/${esc(total)}</span></button>${act}</div>`;
}
/* Filtre courant de l'explorateur : texte (champ #srvFilter) + filtre de tuile
   d'accueil (overviewFilter). Renvoie true si le serveur doit être affiché. */
/* Serveurs masqués de l'affichage — VOLONTAIREMENT en mémoire seulement :
   ni config.json ni localStorage. Fermer l'app (ou recharger la page) les fait
   réapparaître, comme demandé. La supervision moteur continue de les sonder. */
const hiddenServers = new Set();
const selectedServers = new Set();
let lastExpAnchorId = null;
const selectedTools = new Set();
let lastToolAnchor = null;
let expStateFilter = 'all';

function visibleServerIds() {
  return [...document.querySelectorAll('#explorer .grpbody:not(.closed) .srow[data-id]')]
    .map(r => r.dataset.id)
    .filter(Boolean);
}
function handleServerClick(id, ev) {
  if (!id) return;
  const ctrl = !!(ev && (ev.ctrlKey || ev.metaKey));
  const shift = !!(ev && ev.shiftKey);
  if (shift) {
    if (ev && ev.preventDefault) ev.preventDefault();
    try { const sel = window.getSelection && window.getSelection(); if (sel) sel.removeAllRanges(); } catch (e) {}
    const vis = visibleServerIds();
    const anchor = lastExpAnchorId || selectedId || id;
    const i1 = vis.indexOf(anchor);
    const i2 = vis.indexOf(id);
    if (!ctrl) selectedServers.clear();
    if (i1 >= 0 && i2 >= 0) {
      const a = Math.min(i1, i2), b = Math.max(i1, i2);
      for (let i = a; i <= b; i++) selectedServers.add(vis[i]);
    } else {
      selectedServers.add(id);
    }
    renderServers();
    return;
  }
  if (ctrl) {
    if (ev && ev.preventDefault) ev.preventDefault();
    if (selectedServers.size === 0 && selectedId) selectedServers.add(selectedId);
    if (selectedServers.has(id)) selectedServers.delete(id);
    else selectedServers.add(id);
    lastExpAnchorId = id;
    renderServers();
    return;
  }
  selectedServers.clear();
  selectedServers.add(id);
  lastExpAnchorId = id;
  select(id);
}
function visibleToolNames() {
  return [...document.querySelectorAll('#toolsRowsBox .tool[data-tool]')]
    .map(r => r.dataset.tool)
    .filter(Boolean);
}
function handleToolClick(el, ev) {
  if (!detailCache || !el || !el.dataset.tool) return;
  const name = el.dataset.tool;
  const ctrl = !!(ev && (ev.ctrlKey || ev.metaKey));
  const shift = !!(ev && ev.shiftKey);
  if (shift) {
    if (ev && ev.preventDefault) ev.preventDefault();
    try { const sel = window.getSelection && window.getSelection(); if (sel) sel.removeAllRanges(); } catch (e) {}
    const vis = visibleToolNames();
    const anchor = lastToolAnchor || name;
    const i1 = vis.indexOf(anchor);
    const i2 = vis.indexOf(name);
    if (!ctrl) selectedTools.clear();
    if (i1 >= 0 && i2 >= 0) {
      const a = Math.min(i1, i2), b = Math.max(i1, i2);
      for (let i = a; i <= b; i++) selectedTools.add(vis[i]);
    } else {
      selectedTools.add(name);
    }
    refreshToolsBox();
    return;
  }
  if (ctrl) {
    if (ev && ev.preventDefault) ev.preventDefault();
    if (selectedTools.has(name)) selectedTools.delete(name);
    else selectedTools.add(name);
    lastToolAnchor = name;
    refreshToolsBox();
    return;
  }
  selectedTools.clear();
  lastToolAnchor = name;
  toggleToolExpand(el);
}

function setExpStateFilter(mode) {
  expStateFilter = mode || 'all';
  overviewFilter = expStateFilter === 'up' ? 'up' : null;
  document.querySelectorAll('#expStatePills .exp-pill').forEach(b => {
    const sel = b.dataset.mode === expStateFilter;
    b.classList.toggle('sel', sel);
    b.setAttribute('aria-pressed', sel ? 'true' : 'false');
  });
  renderServers();
  if (!selectedId) renderOverview();
}

function setExplorerTab(tab) {
  const next = tab === 'swarm' ? 'swarm' : 'mcp';
  explorerTab = next;
  const bMcp = document.getElementById('explTabMcp');
  const bSw = document.getElementById('explTabSwarm');
  if (bMcp) { bMcp.classList.toggle('sel', next === 'mcp'); bMcp.setAttribute('aria-selected', next === 'mcp' ? 'true' : 'false'); }
  if (bSw) { bSw.classList.toggle('sel', next === 'swarm'); bSw.setAttribute('aria-selected', next === 'swarm' ? 'true' : 'false'); }
  _sigServers = '';
  if (next === 'swarm') {
    openSwarmView();
  } else {
    if (activeView === 'swarm') {
      activeView = 'dashboard';
      if (typeof swarmStopPoll === 'function') swarmStopPoll();
      render();
    } else {
      renderServers();
    }
  }
}
function srvVisible(s, q) {
  if (hiddenServers.has(s.id)) return false;
  if (explorerTab === 'swarm' && !s.swarm_kind) return false;
  if (overviewFilter === 'up' && s.state !== 'UP') return false;
  if (expStateFilter === 'up' && s.state !== 'UP') return false;
  if (expStateFilter === 'down' && !(s.enabled && (s.state === 'DOWN' || s.state === 'PORT-ONLY') && !(s.locked && !s.paired))) return false;
  if (expStateFilter === 'locked' && !(s.locked && !s.paired)) return false;
  if (expStateFilter === 'paused' && (s.enabled && s.state !== 'DISABLED')) return false;
  if (expStateFilter === 'swarm' && !s.swarm_kind) return false;
  if (q) {
    const hay = (s.label + ' ' + s.id + ' ' + (s.framework || '') + ' '
      + (s.swarm_kind ? ('swarm swarm-control ' + s.swarm_kind + ' ' + (s.swarm_agent || '') + ' resident ') : '')
      + (s.in_container ? ('docker conteneur container ' + (s.container_name || '') + ' ') : '')
      + (s.transport === 'http' ? (s.transport_info.port ?? '') : '') + ' ' + (s.state || '')).toLowerCase();
    if (!hay.includes(q)) return false;
  }
  return true;
}
function renderServers() {
  const explEl = document.getElementById('explorer');
  const localBox = document.getElementById('srvList');
  const groupsBox = document.getElementById('remoteGroups');
  const label = document.getElementById('remoteLabel');
  const count = document.getElementById('localCount');
  const localHead = document.getElementById('localHead');
  const clr = document.getElementById('srvFilterClear');
  const qRaw = (document.getElementById('srvFilter')?.value || '').trim();
  const q = qRaw.toLowerCase();
  if (clr) setHidden(clr, !qRaw);
  const keys = groupKeys();
  const sig = JSON.stringify([
    state.servers,
    state.swarm_stats,
    state.swarm_logos_rev,
    state.settings && state.settings.ssh_hosts,
    selectedId,
    activeView,
    explorerTab,
    [...selectedServers],
    overviewFilter,
    expStateFilter,
    q,
    [...hiddenServers],
    keys.map(k => [k, groupClosed(k)]),
    groupClosed('local'),
    Object.keys(FRAMEWORKS).length
  ]);
  if (sig === _sigServers) return;
  _sigServers = sig;
  const active = document.activeElement;
  let focusSpec = null;
  if (explEl && active && explEl.contains(active)) {
    const srow = active.closest ? active.closest('.srow[data-id]') : null;
    if (active.matches && active.matches('.srow[data-id]')) {
      focusSpec = {kind: 'srow', id: active.dataset.id};
    } else if (srow && active.dataset && active.dataset.act) {
      focusSpec = {kind: 'rowbtn', id: srow.dataset.id, act: active.dataset.act, op: active.dataset.op || ''};
    } else if (active.matches && active.matches('.grp-toggle[data-key]') && active.dataset.key !== 'local') {
      focusSpec = {kind: 'grp', key: active.dataset.key};
    }
  }
  const hiveMatchesFilter = (explorerTab === 'swarm' || expStateFilter === 'swarm')
    && (!q || 'essaim swarm a2a orchestration agents'.includes(q));
  const hiveHtml = hiveMatchesFilter ? swarmHiveRow() : '';
  if (!state.servers.length) {
    setHidden(localHead, false); setText(count, '0/0');
    localBox.innerHTML = hiveHtml + '<div class="empty-sec" role="listitem">aucun serveur \u2014 lance un scan</div>';
    setHidden(label, true); groupsBox.innerHTML = '';
    const hb = document.getElementById('hiddenBar');
    if (hb) setHidden(hb, true);
    return;
  }
  const shown = state.servers.filter(s => !hiddenServers.has(s.id));
  const localAll = shown.filter(s => hostOf(s) === 'local');
  const local = localAll.filter(s => srvVisible(s, q));
  const remotes = shown.filter(s => hostOf(s) !== 'local');
  const byKey = {};
  remotes.forEach(s => { const k = hostOf(s); (byKey[k] = byKey[k] || []).push(s); });
  const hb = document.getElementById('hiddenBar');
  if (hb) {
    const n = hiddenServers.size;
    setHidden(hb, !n);
    if (n) hb.innerHTML = ic('eyeOff') + ' ' + esc(n) + ' serveur' + (n > 1 ? 's' : '')
      + ' masqu\u00e9' + (n > 1 ? 's' : '') + ' (' + (n > 1 ? 'reviennent' : 'revient') + ' \u00e0 la r\u00e9ouverture) <b>afficher</b>';
  }
  const upOf = rows => rows.filter(s => s.state === 'UP').length;
  const favFirst = rows => rows.slice().sort((a, b) => ((b.favorite === true) - (a.favorite === true)));
  const noneMatch = '<div class="empty-sec" role="listitem">aucun serveur ne correspond au filtre</div>';
  setHidden(localHead, false);
  const locClosed = groupClosed('local');
  localHead.classList.toggle('closed', locClosed);
  const localToggle = localHead.querySelector('.grp-toggle');
  if (localToggle) localToggle.setAttribute('aria-expanded', locClosed ? 'false' : 'true');
  localBox.classList.toggle('closed', locClosed);
  const localScope = explorerTab === 'swarm'
    ? localAll.filter(s => Boolean(s.swarm_kind))
    : localAll;
  setText(count, upOf(localScope) + '/' + localScope.length);
  localBox.innerHTML = hiveHtml + (local.length ? favFirst(local).map(srvRow).join('')
    : (localScope.length ? (hiveHtml ? '' : noneMatch)
      : (explorerTab === 'swarm'
        ? (hiveHtml ? '' : '<div class="empty-sec" role="listitem">aucun serveur Swarm d&eacute;tect&eacute;</div>')
        : '<div class="empty-sec" role="listitem">aucun serveur \u2014 lance un scan</div>')));
  const visibleRemoteKeys = explorerTab === 'swarm'
    ? keys.filter(k => (byKey[k] || []).some(s => Boolean(s.swarm_kind)))
    : keys;
  setHidden(label, !visibleRemoteKeys.length);
  groupsBox.innerHTML = visibleRemoteKeys.map((k, idx) => {
    const all = explorerTab === 'swarm'
      ? (byKey[k] || []).filter(s => Boolean(s.swarm_kind))
      : (byKey[k] || []);
    const rows = all.filter(s => srvVisible(s, q));
    const bodyId = 'grpBody_' + idx;
    const body = rows.length ? favFirst(rows).map(srvRow).join('')
      : (all.length ? noneMatch
        : '<div class="empty-sec empty-indent" role="listitem">aucun serveur import&eacute; de cette machine</div>');
    return groupHeadHtml(k, k, upOf(all), all.length, true, bodyId)
      + `<div id="${esc(bodyId)}" class="grpbody mbody ${groupClosed(k)?'closed':''}" role="list" aria-label="Serveurs sur ${esc(k)}">${body}</div>`;
  }).join('');
  if (focusSpec && explEl) {
    let target = null;
    if (focusSpec.kind === 'srow') {
      target = [...explEl.querySelectorAll('.srow[data-id]')].find(r => r.dataset.id === focusSpec.id) || null;
    } else if (focusSpec.kind === 'rowbtn') {
      const row = [...explEl.querySelectorAll('.srow[data-id]')].find(r => r.dataset.id === focusSpec.id);
      if (row) {
        const btns = [...row.querySelectorAll('button[data-act]')];
        target = (focusSpec.op && btns.find(b => b.dataset.act === focusSpec.act && b.dataset.op === focusSpec.op))
          || btns.find(b => b.dataset.act === focusSpec.act)
          || row;
      }
    } else if (focusSpec.kind === 'grp') {
      target = [...explEl.querySelectorAll('.grp-toggle[data-key]')].find(g => g.dataset.key === focusSpec.key) || null;
    }
    if (target && document.activeElement !== target) {
      try { target.focus({preventScroll: true}); } catch (e) {}
    }
  }
}

// Chip du langage MCP (inféré : commande stdio ou en-têtes HTTP) — monogramme coloré façon logo
const LANG_META = {
  node:   { t: 'JS',  c: '#dcc94f', n: 'Node.js / TypeScript' },
  python: { t: 'PY',  c: '#3fb950', n: 'Python' },
  rust:   { t: 'RS',  c: '#f97316', n: 'Rust' },
  go:     { t: 'GO',  c: '#00add8', n: 'Go' },
  java:   { t: 'JV',  c: '#e76f00', n: 'Java' },
  dotnet: { t: 'C#',  c: '#512bd4', n: '.NET' },
  ruby:   { t: 'RB',  c: '#cc342d', n: 'Ruby' },
  php:    { t: 'PHP', c: '#777bb4', n: 'PHP' },
};
// Glyphe Rust : « R » dans un engrenage (lecture de l'icône .rs des éditeurs).
// NB : attributs en simples quotes — guillemet-double suivi de dièse interdit
// dans ce bloc (cette séquence fermerait la raw string Rust d'INDEX_HTML).
const RUST_LOGO = "<svg class='rustlogo' viewBox='0 0 24 24' aria-hidden='true'><g stroke='#f97316' stroke-width='1.4' stroke-linecap='round' fill='none'><circle cx='12' cy='12' r='7.9'/><path d='M19.9 12h2M17.6 17.6l1.4 1.4M12 19.9v2M6.4 17.6 5 19M4.1 12h-2M6.4 6.4 5 5M12 4.1v-2M17.6 6.4 19 5'/></g><text x='12' y='16.2' text-anchor='middle' font-size='12' font-weight='800' font-family='inherit' fill='#f97316'>R</text></svg>";
/* Logos langages embarqués (marques officielles en SVG minimal) :
   JS = carré jaune officiel ECMAScript ; Python = les deux serpents (la forme
   bleue est répétée pivotée de 180°, comme le vrai logo) ; Go = tuile cyan
   officielle ; .NET = carré violet officiel ; Java = la tasse fumante ;
   Ruby = le gem facetté ; PHP = l'ellipse violette. NB : attributs en simples
   quotes — guillemet-double suivi de dièse interdit dans ce bloc (fermerait la
   raw string Rust d'INDEX_HTML). */
const PY_PATH = "M11.8 2.1c-2.6 0-4.7 1.1-4.7 3.2v2.6h4.9v.9H5.1c-2.1 0-3.8 1.6-3.8 4.4 0 2.9 1.5 4.5 3.6 4.5h2v-2.8c0-1.9 1.6-3.5 3.5-3.5h4.4c1.6 0 2.9-1.3 2.9-2.9V5.3c0-2.1-2.1-3.2-4.7-3.2h-1.2z";
const LANG_LOGOS = {
  rust: RUST_LOGO,
  node: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><rect x='1.5' y='1.5' width='21' height='21' rx='3.5' fill='#dcc94f'/><text x='12' y='16.6' text-anchor='middle' font-size='10.5' font-weight='800' font-family='inherit' fill='#221e08'>JS</text></svg>",
  python: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><g><path fill='#3776ab' d='" + PY_PATH + "'/><circle cx='9.7' cy='5' r='.95' fill='#fff'/><g transform='rotate(180 12 12)'><path fill='#ffd43b' d='" + PY_PATH + "'/><circle cx='9.7' cy='5' r='.95' fill='#fff'/></g></g></svg>",
  go: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><rect x='1.5' y='1.5' width='21' height='21' rx='3.5' fill='#00add8'/><text x='12' y='16.2' text-anchor='middle' font-size='10' font-weight='800' font-family='inherit' fill='#fff'>GO</text></svg>",
  java: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><path fill='#f89820' d='M12.4 2.4c-.6 1.4.1 2.4 1.2 3.1 1.1.8 1.7 1.8 1.3 3 1.2-.7 1.6-2.1.8-3.3-.6-1-2-1.7-3.3-2.8z'/><path fill='#f89820' d='M9.4 7c-.5 1.1.1 2 1.1 2.5.9.6 1.4 1.4 1 2.4 1.1-.6 1.5-1.8.8-2.8-.6-.9-1.8-1.4-2.9-2.1z'/><path fill='#5382a1' d='M7.2 12.1h9.3v4.7c0 2.3-1.9 4.2-4.2 4.2h-.9c-2.3 0-4.2-1.9-4.2-4.2z'/><path fill='none' stroke='#5382a1' stroke-width='1.3' stroke-linecap='round' d='M16.9 13.4h1.2c1.1 0 2 .9 2 2s-.9 2-2 2h-1.3'/></svg>",
  dotnet: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><rect x='1.5' y='1.5' width='21' height='21' rx='3.5' fill='#512bd4'/><text x='12' y='15.4' text-anchor='middle' font-size='6.4' font-weight='700' font-family='inherit' fill='#fff'>.NET</text></svg>",
  ruby: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><path fill='#cc342d' d='M7.2 4.5h9.6l3.7 4.6L12 20.5 3.5 9.1z'/><path fill='#e9655c' d='M7.2 4.5h9.6L12 9.5z'/><path fill='#a72b24' d='M3.5 9.1h17L12 20.5z' opacity='.4'/><path fill='none' stroke='#fff' stroke-opacity='.4' stroke-width='.7' d='M7.2 4.5 12 9.5l4.8-5M12 9.5v11M3.5 9.1h17'/></svg>",
  php: "<svg class='langlogo' viewBox='0 0 24 24' aria-hidden='true'><ellipse cx='12' cy='12' rx='10.6' ry='6.2' fill='#777bb4'/><text x='12' y='14.2' text-anchor='middle' font-size='6.2' font-weight='700' font-style='italic' font-family='inherit' fill='#fff'>php</text></svg>",
};
function langChip(s) {
  const logo = LANG_LOGOS[s.lang];
  if (!logo) return '<span class="langchip unk" aria-label="Langage non d\u00e9tect\u00e9" title="Langage non d\u00e9tect\u00e9 (ni commande ni en-t\u00eates ni process)">?</span>';
  const m = LANG_META[s.lang];
  const name = m ? m.n : s.lang;
  return `<span class="rustwrap" title="Langage inféré : ${esc(name)}">${logo}</span>`;
}

// Petit logo Docker (baleine officielle) lorsque le serveur MCP tourne dans un conteneur.
const DOCKER_LOGO = "<svg class='dockerlogo' viewBox='0 0 24 24' aria-hidden='true'><path fill='#2496ED' d='M13.983 11.078h2.119a.186.186 0 00.186-.185V9.006a.186.186 0 00-.186-.186h-2.119a.185.185 0 00-.185.185v1.888c0 .102.083.185.185.185m-2.954-5.43h2.118a.186.186 0 00.186-.186V3.574a.186.186 0 00-.186-.185h-2.118a.185.185 0 00-.185.185v1.888c0 .102.082.185.185.186m0 2.716h2.118a.187.187 0 00.186-.186V6.29a.186.186 0 00-.186-.185h-2.118a.185.185 0 00-.185.185v1.887c0 .102.082.185.185.186m-2.93 0h2.12a.186.186 0 00.184-.186V6.29a.185.185 0 00-.185-.185H8.1a.185.185 0 00-.185.185v1.887c0 .102.083.185.185.186m-2.964 0h2.119a.186.186 0 00.185-.186V6.29a.185.185 0 00-.185-.185H5.136a.186.186 0 00-.186.185v1.887c0 .102.084.185.186.186m5.893 2.715h2.118a.186.186 0 00.186-.185V9.006a.186.186 0 00-.186-.186h-2.118a.185.185 0 00-.185.185v1.888c0 .102.082.185.185.185m-2.93 0h2.12a.185.185 0 00.184-.185V9.006a.185.185 0 00-.184-.186h-2.12a.185.185 0 00-.184.185v1.888c0 .102.083.185.185.185m-2.964 0h2.119a.185.185 0 00.185-.185V9.006a.185.185 0 00-.185-.186H5.136a.186.186 0 00-.186.186v1.887c0 .102.084.185.186.185m-2.92 0h2.12a.185.185 0 00.184-.185V9.006a.185.185 0 00-.184-.186h-2.12a.186.186 0 00-.185.186v1.887c0 .102.084.185.185.185M23.763 9.89c-.065-.051-.672-.51-1.954-.51-.338.001-.676.03-1.01.087-.248-1.7-1.653-2.53-1.716-2.566l-.344-.199-.226.327c-.284.438-.49.922-.612 1.43-.23.97-.09 1.882.403 2.661-.595.332-1.55.413-1.744.42H.751a.751.751 0 00-.75.748 11.376 11.376 0 00.692 4.062c.545 1.428 1.355 2.48 2.41 3.124 1.18.723 3.1 1.137 5.275 1.137.983.003 1.963-.086 2.93-.266a12.248 12.248 0 003.823-1.389c.98-.567 1.86-1.288 2.61-2.136 1.252-1.418 1.998-2.997 2.553-4.4h.221c1.372 0 2.215-.549 2.68-1.009.309-.293.55-.65.707-1.046l.098-.288Z'/></svg>";
function dockerBadge(s, verbose) {
  if (!s || !s.in_container) return '';
  const tip = s.container_name
    ? 'Conteneur Docker (' + s.container_name + ')'
    : 'Ex\u00e9cut\u00e9 dans un conteneur Docker';
  if (!verbose) {
    return `<span class="dockerwrap" role="img" aria-label="${esc(tip)}" title="${esc(tip)}">${DOCKER_LOGO}</span>`;
  }
  const lbl = s.container_name ? 'Docker \u00b7 ' + s.container_name : 'Docker';
  return `<span class="tag docker" title="${esc(tip)}">${DOCKER_LOGO}<span>${esc(lbl)}</span></span>`;
}

// Badge du type de MCP : STDIO / HTTP / WS — trois modes, point.
// HTTP couvre le streamable JSON et les réponses encodées SSE (FastMCP).
function typeBadge(s) {
  // Trois modes affichés, point : STDIO / HTTP / WS. Le streamable JSON et les
  // réponses encodées SSE (FastMCP) sont tous les deux « HTTP » — pas de badge
  // séparé. Le framework (LiteMCP/FastMCP) a son propre badge.
  if (s.transport === 'stdio') return '<span class="tag stdio" title="Transport stdio (process local)">STDIO</span>';
  const h = s.transport_info && s.transport_info.host;
  const lan = (h && !isLoopbackHost(h)) ? '\u00b7LAN' : '';
  const via = s.via_ssh ? '\u00b7SSH' : '';
  const viaTip = s.via_ssh ? ' — sondé via SSH depuis la machine (interne)' : '';
  const suffix = (lan ? ' — serveur distant (LAN)' : '') + viaTip;
  if (s.ws) return `<span class="tag ws" title="WebSocket — le serveur accepte une upgrade WS (101)${suffix}">WS${lan}${via}</span>`;
  return `<span class="tag http" title="HTTP streamable${suffix}">HTTP${lan}${via}</span>`;
}

// Badge du framework détecté par fingerprint :
// - litemcp : initialize répondu 2026-07-28 SANS négocier + sonde /health « ok » ;
// - fastmcp : réponses initialize encodées SSE + endpoint /sse legacy co-servi
//   (signature httpStream de mcp-proxy/FastMCP). La spec MCP utilisée par le
//   serveur (s.protocol) est exposée dans le tooltip et le détail.
// Table des frameworks MCP (id → {label, lang}) chargée depuis /api/frameworks :
// une seule source de vérité côté Rust, pas de liste dupliquée en JS.
let FRAMEWORKS = {};
async function loadFrameworks() {
  try {
    const r = await fetch('/api/frameworks');
    if (!r.ok) return;
    const j = await r.json();
    (j.frameworks || []).forEach(f => { FRAMEWORKS[f.id] = f; });
    if (state) { render(); if (selectedId && detailCache) renderDetail(); }
  } catch (e) { /* hors ligne : les badges afficheront l'id brut */ }
}
function frameworkBadge(s) {
  if (!s.framework) return '';
  const f = FRAMEWORKS[s.framework] || {label: s.framework, lang: s.lang || '?'};
  const h = s.transport_info && s.transport_info.host;
  const lan = (h && !isLoopbackHost(h)) ? ' — serveur distant (LAN)' : '';
  const tip = `Framework ${esc(f.label)} · ${esc(f.lang)}${lan}`;
  // LiteMCP est le seul badge qui garde un halo (classe .litemcp, animation glow).
  if (s.framework === 'litemcp') {
    return `<span class="tag litemcp" title="${tip}">${esc(f.label)}</span>`;
  }
  // Les autres badges prennent la couleur du LANGAGE du framework, comme les
  // puces de code : la classe .fw-<lang> porte couleur, bordure et fond (fin des
  // styles en ligne — CSP style-src 'self').
  const lk = LANG_META[f.lang] ? f.lang : (LANG_META[s.lang] ? s.lang : '');
  if (!lk) return `<span class="tag fw" title="${tip}">${esc(f.label)}</span>`;
  return `<span class="tag fw fw-${lk}" title="${tip}">${esc(f.label)}</span>`;
}

// Détecte si les variables d'environnement d'identité MCP Control sont injectées (UI-C01).
// `json.rs` sérialise `env` en tableau de noms (`["LITEMCP_TRUSTED_KEYS", ...]`).
function hasAuthEnv(s) {
  const envs = [
    s && s.start_cmd && s.start_cmd.env,
    s && s.transport_info && s.transport_info.env,
  ];
  return envs.some(e => {
    if (!e || typeof e !== 'object') return false;
    if (Array.isArray(e)) {
      return e.includes('LITEMCP_TRUSTED_KEYS') || e.includes('MCC_AUTH_PUBKEY');
    }
    return 'LITEMCP_TRUSTED_KEYS' in e || 'MCC_AUTH_PUBKEY' in e;
  });
}

// Badge cadenas + badge AUTH : serveur verrouillé / appairé RFC 9421 ou env d'auth injecté.
function lockBadge(s, verbose) {
  if (!s) return '';
  let out = '';
  if (s.locked || s.paired) {
    const paired = !!s.paired;
    const tip = paired
      ? 'Auth MCP Control active (signature RFC 9421, cl\u00e9 ' + (s.auth_keyid || 'mcc-server') + ')'
      : 'Serveur verrouill\u00e9 (401 Signature) \u2014 clic droit ou bouton Appairer pour l\u2019associer \u00e0 MCP Control';
    const lbl = verbose ? (paired ? ' Appair&eacute;' : ' Verrouill&eacute;') : '';
    out += `<span class="tag lock ${paired ? 'paired' : 'unpaired'}" title="${esc(tip)}">${ic('lock')}${lbl}</span>`;
  }
  if (hasAuthEnv(s)) {
    out += `<span class="tag auth-env" title="Variables d&#39;identit&eacute; Ed25519 MCP Control (LITEMCP_TRUSTED_KEYS / MCC_AUTH_PUBKEY) inject&eacute;es au lancement">AUTH</span>`;
  }
  return out;
}

function rowFav(s) {
  const on = !!s.favorite;
  return `<button type="button" class="rowbtn fav ${on?'on':'off'}" aria-label="${on ? 'Retirer des favoris' : 'Ajouter aux favoris'} ${esc(s.label)}" title="${on
    ? 'Retirer des favoris (l\u2019\u00e9pingle maintient la ligne en haut de sa section)'
    : '\u00c9pingler en favori : la ligne passe en haut de sa section, quel que soit le tri'}"
    data-act="set-fav" data-id="${esc(s.id)}" data-on="${on ? '0' : '1'}">${ic('star')}</button>`;
}

function rowToggle(s) {
  return `<button type="button" class="sw ${s.enabled?'on':'off'}" role="switch" aria-checked="${s.enabled?'true':'false'}" aria-label="${s.enabled?'D\u00e9sactiver':'Activer'} ${esc(s.label)}" title="${s.enabled?'D\u00e9sactiver ce serveur (arr\u00eate son process)':'Activer ce serveur (le d\u00e9marre si commande connue)'}" data-act="row-act" data-op="toggle" data-id="${esc(s.id)}">${s.enabled?'ON':'OFF'}</button>`;
}

function rowCtl(s) {
  if (!s.can_start) return '';
  const up = s.state === 'UP';
  return `<button type="button" class="rowbtn" aria-label="${up?'Arr\u00eater':'D\u00e9marrer'} ${esc(s.label)}" title="${up?'Arr\u00eater':'D\u00e9marrer'} ce serveur" data-act="row-act" data-op="${up?'kill':'start'}" data-id="${esc(s.id)}">${up?ic('stop'):ic('play')}</button>`;
}

// Autoboot PAR SERVEUR : relance automatique de CE serveur au démarrage
// de MCP Control et après un crash. Bouton ⚡ visible dans la ligne et le détail.
function rowAuto(s) {
  if (!s.can_start) return '';
  const on = s.auto_restart;
  return `<button type="button" class="rowbtn auto ${on?'on':'off'}" aria-label="Autoboot ${on?'activ\u00e9':'d\u00e9sactiv\u00e9'} pour ${esc(s.label)}" title="${on
    ? 'Autoboot ACTIV\u00c9 : ce serveur est relanc\u00e9 au d\u00e9marrage et apr\u00e8s un crash \u2014 cliquer pour d\u00e9sactiver'
    : 'Autoboot D\u00c9SACTIV\u00c9 : ce serveur ne sera pas relanc\u00e9 automatiquement \u2014 cliquer pour activer'}"
    data-act="set-auto" data-id="${esc(s.id)}" data-on="${on ? '0' : '1'}">${ic('zap')}</button>`;
}

/* État des replis du détail (mémorisé entre les cycles et les sessions) */
function saveFold(el) {
  try { localStorage.setItem('mcc.fold.' + el.dataset.fold, el.open); } catch (e) {}
}

async function toggleAutostart() {
  const next = !state.autostart;
  const r = await postAction({action: 'set_autostart', enabled: next}, {quiet: true});
  if (r && r.needs_confirm) {
    const ok = await askConfirm({
      title: 'Remplacer le d\u00e9marrage automatique',
      msg: (r.error || 'Une autre installation est d\u00e9j\u00e0 enregistr\u00e9e au d\u00e9marrage.')
        + (r.recorded ? '\n\nChemin actuel : ' + r.recorded : ''),
      okLabel: 'Remplacer',
      danger: true,
    });
    if (ok) {
      await postAction({action: 'set_autostart', enabled: next, confirm: true});
    }
  } else if (r && !r.ok && r.error) {
    toast(r.error, 'err', 4000);
  }
  await load();
}

/* ===== Drag & drop : réordonner les serveurs (préférence / priorité) ===== */
let dragId = null;
function rowDragStart(ev, el, id) {
  dragId = id;
  ev.dataTransfer.effectAllowed = 'move';
  try { ev.dataTransfer.setData('text/plain', id); } catch (e) {}
  el.classList.add('dragging');
}
function rowDragOver(ev, el) {
  if (!dragId) return;
  const dragged = byId(dragId);
  // un serveur distant ne se déplace pas dans la zone plate locale
  if (!dragged || hostOf(dragged) !== 'local') return;
  ev.preventDefault();
  ev.dataTransfer.dropEffect = 'move';
  const row = el;
  if (row.dataset.id === dragId) return;
  const rect = row.getBoundingClientRect();
  const above = ev.clientY < rect.top + rect.height / 2;
  row.classList.toggle('drop-above', above);
  row.classList.toggle('drop-below', !above);
  row._dropAbove = above;
}
function rowDragLeave(ev, el) {
  el.classList.remove('drop-above', 'drop-below');
}
function rowDrop(ev, el) {
  ev.preventDefault();
  const row = el;
  row.classList.remove('drop-above', 'drop-below');
  const moved = dragId;
  dragId = null;
  if (!moved || row.dataset.id === moved) return;
  const dragged = byId(moved);
  // un serveur distant ne se déplace pas dans la zone plate locale
  if (!dragged || hostOf(dragged) !== 'local') return;
  // le glisser-déposer reprend la main : le menu de tri revient sur « manuel »
  manualOrder = null;
  const sel = document.getElementById('sortSel');
  if (sel) sel.value = 'manuel';
  // nouvel ordre = ids visibles, en conservant les serveurs masqués par filtre
  const visIds = [...document.querySelectorAll('#srvList .srow')]
    .map(r => r.dataset.id).filter(x => x && x !== moved);
  const at = visIds.indexOf(row.dataset.id);
  if (at < 0) return;
  visIds.splice(row._dropAbove ? at : at + 1, 0, moved);
  const visSet = new Set(visIds);
  const allIds = (state && state.servers ? state.servers : []).map(s => s.id);
  let vi = 0;
  const merged = allIds.map(id => visSet.has(id) ? visIds[vi++] : id);
  postReorder(merged);
}
/* Drop d'une ligne de serveur sur l'EN-TÊTE d'un groupe machine : re-host du
   serveur (sa clé de groupe = le host du groupe). Cas d'usage : une machine a
   changé d'IP, ou une importation a posé le mauvais host — on déplace la ligne
   vers la bonne machine sans réimporter. */
function groupDragOver(ev, el) {
  if (!dragId) return;
  ev.preventDefault();
  ev.dataTransfer.dropEffect = 'move';
  el.classList.add('drag-over-group');
}
function groupDragLeave(ev, el) {
  el.classList.remove('drag-over-group');
}
async function groupDrop(ev, el, key) {
  ev.preventDefault();
  el.classList.remove('drag-over-group');
  const moved = dragId;
  dragId = null;
  if (!moved) return;
  const s = byId(moved);
  if (!s || s.transport !== 'http') { toast('Seuls les serveurs HTTP peuvent \u00eatre h\u00e9berg\u00e9s par une machine distante.', 'err', 4000); return; }
  const ok = await askConfirm({
    title: 'D\u00e9placer vers une machine distante',
    msg: 'H\u00e9berger \u00ab ' + (s.label || moved) + ' \u00bb sur ' + key + ' ?',
    detail: 'Le host du transport HTTP deviendra ' + key + '.',
    okLabel: 'D\u00e9placer',
  });
  if (!ok) return;
  const j = await postAction({action: 'set_host', id: moved, host: key}, {quiet: true});
  if (!j.ok) toast('set_host : ' + (j.error || '\u00e9chec'), 'err', 4000);
  await load();
}

/* ===== Menu de reclassement (⇅) =====
   Réutilise l'action API « reorder » : l'ordre appliqué est persisté
   (settings.server_order) et gouverne l'affichage ET la priorité de sondage.
   « manuel » rend l'ordre posé au glisser-déposer AVANT le tri (mémorisé côté
   client au moment où un tri est appliqué — un drag le réactualise). */
let manualOrder = null;
const SORT_STATE_RANK = {UP: 0, STARTING: 1, 'PORT-ONLY': 1, PROBING: 1, DOWN: 2, DISABLED: 3};
const frCmp = (a, b) => String(a).localeCompare(String(b), 'fr', {sensitivity: 'base'});
function sortByPort(s) {
  return s.transport === 'http' ? (s.transport_info.port || 65536) : 65536;
}
function applySort(mode) {
  if (!state || !state.servers.length) return;
  const byId = Object.fromEntries(state.servers.map(s => [s.id, s]));
  if (mode === 'manuel') {
    if (manualOrder && manualOrder.length) postReorder(manualOrder.filter(id => byId[id]));
    return;
  }
  if (manualOrder === null) manualOrder = state.servers.map(s => s.id);
  // Tri à plat sur l'ordre persisté — chaque groupe (locaux, puis machines)
  // affiche ses serveurs dans cet ordre commun, jamais un tri global mélangé.
  let ids = state.servers.map(s => s.id);
  if (mode === 'az') {
    ids.sort((a, b) => frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'etat') {
    ids.sort((a, b) => ((SORT_STATE_RANK[byId[a].state] ?? 9) - (SORT_STATE_RANK[byId[b].state] ?? 9))
      || frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'port') {
    ids.sort((a, b) => (sortByPort(byId[a]) - sortByPort(byId[b])) || frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'latence') {
    // non sondés (null) en fin, puis du plus rapide au plus lent
    ids.sort((a, b) => ((byId[a].latency_ms ?? 1e12) - (byId[b].latency_ms ?? 1e12)));
  } else if (mode === 'tools') {
    ids.sort((a, b) => ((byId[b].tools_count || 0) - (byId[a].tools_count || 0))
      || frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'framework') {
    // Regroupe par framework MCP (label humain), les serveurs sans framework
    // identifiable en fin de liste. À l'intérieur d'un framework : A→Z.
    const fwKey = s => {
      const f = FRAMEWORKS[s.framework];
      return f ? f.label : (s.framework || '\uffff');
    };
    ids.sort((a, b) => frCmp(fwKey(byId[a]), fwKey(byId[b]))
      || frCmp(byId[a].label, byId[b].label));
  } else if (mode === 'transport') {
    // stdio → HTTP → WS (WS est un HTTP avec upgrade), puis A→Z.
    const tRank = s => (s.transport === 'stdio' ? 0 : (s.ws ? 2 : 1));
    ids.sort((a, b) => (tRank(byId[a]) - tRank(byId[b]))
      || frCmp(byId[a].label, byId[b].label));
  }
  postReorder(ids);
}
async function postReorder(ids) {
  await postAction({action: 'reorder', ids});
  await load();
}
function rowDragEnd(ev, el) {
  el.classList.remove('dragging');
  document.querySelectorAll('.srow').forEach(r => r.classList.remove('drop-above', 'drop-below'));
  dragId = null;
}

function renderPeers() {
  const box = document.getElementById('peerList');
  const sig = JSON.stringify(state.peers || []);
  if (sig === _sigPeers) return;
  _sigPeers = sig;
  if (!state.peers || !state.peers.length) { box.innerHTML = '<div class="empty-sec">aucun pair d&eacute;tect&eacute;</div>'; return; }
  box.innerHTML = state.peers.map(p => {
    const st = p.stats || {};
    return `<div class="peer-row">
      <div class="l1"><span class="dot UP"></span><span class="nm">${esc(p.ip)}</span></div>
      <div class="l2">UI :${esc(p.ui_port)} &mdash; ${esc(st.up ?? '?')} UP / ${esc(st.total ?? '?')} serveurs</div>
      <button data-act="import-peer" data-id="${esc(p.ip)}">Importer ses serveurs</button>
    </div>`;
  }).join('');
}

let logFilterLevel = 'all';
let logClearWatermark = null;
let logClearTs = 0;
let _logInitialized = false;
function logSigOf(l) {
  return `${Number.isFinite(l.ts) ? l.ts : 0}|${l.level || ''}|${l.msg || ''}`;
}
function getVisibleLogs(ignoreLevel) {
  const chrono = [...((state && state.log) || [])].reverse();
  let afterClear = chrono;
  if (logClearWatermark && logClearWatermark.length) {
    const anchor = logClearWatermark[logClearWatermark.length - 1];
    let matchIdx = -1;
    for (let i = chrono.length - 1; i >= 0; i--) {
      if (logSigOf(chrono[i]) !== anchor) continue;
      let ok = true;
      for (let k = 1; k < logClearWatermark.length && i - k >= 0; k++) {
        if (logSigOf(chrono[i - k]) !== logClearWatermark[logClearWatermark.length - 1 - k]) {
          ok = false; break;
        }
      }
      if (ok) { matchIdx = i; break; }
    }
    if (matchIdx >= 0) {
      afterClear = chrono.slice(matchIdx + 1);
    } else if (chrono.length && (chrono[chrono.length - 1].ts || 0) <= logClearTs) {
      afterClear = [];
    }
  }
  if (ignoreLevel || logFilterLevel === 'all') return afterClear;
  return afterClear.filter(l => l.level === logFilterLevel);
}
function renderLog() {
  const lb = document.getElementById('logbody');
  if (!lb) return;
  const sig = JSON.stringify([state.log || [], logFilterLevel, logClearWatermark]);
  if (sig === _sigLog) return;
  _sigLog = sig;
  const allVis = getVisibleLogs(true);
  const filtered = logFilterLevel === 'all' ? allVis : allVis.filter(l => l.level === logFilterLevel);
  const countBadge = document.getElementById('logCountBadge');
  const errBadge = document.getElementById('logErrBadge');
  const errCount = allVis.filter(l => l.level === 'error').length;
  const warnCount = allVis.filter(l => l.level === 'warn').length;
  if (countBadge) {
    setHidden(countBadge, !allVis.length);
    setText(countBadge, allVis.length);
    countBadge.title = `${allVis.length} entr\u00e9e(s)${warnCount ? ' dont ' + warnCount + ' alerte(s)' : ''}`;
  }
  if (errBadge) {
    setHidden(errBadge, !errCount);
    setText(errBadge, errCount);
    errBadge.title = `${errCount} erreur(s)`;
  }
  const wasAtBottom = !_logInitialized || (lb.scrollHeight - lb.scrollTop - lb.clientHeight) <= 16;
  _logInitialized = true;
  lb.innerHTML = filtered.length
    ? filtered.map(l => {
        const ts = Number.isFinite(l.ts) ? l.ts : 0;
        const d = new Date(ts * 1000).toLocaleTimeString('fr-FR');
        return `<div class="logline ${esc(l.level)}"><span class="t">${esc(d)}</span>  ${esc(l.msg)}</div>`;
      }).join('')
    : `<div class="empty-note">Aucune entr&eacute;e${logFilterLevel !== 'all' ? ' pour ce niveau' : ''}.</div>`;
  if (wasAtBottom) lb.scrollTop = lb.scrollHeight;
}
function setLogFilter(level) {
  logFilterLevel = level || 'all';
  document.querySelectorAll('#logFilters .log-fbtn').forEach(b => {
    const sel = b.dataset.level === logFilterLevel;
    b.classList.toggle('sel', sel);
    b.setAttribute('aria-pressed', sel ? 'true' : 'false');
  });
  renderLog();
}
function copyVisibleLog() {
  const rows = getVisibleLogs(false);
  if (!rows.length) { toast('Aucune ligne visible \u00e0 copier', 'warn'); return; }
  const lines = rows.map(l => {
    const ts = Number.isFinite(l.ts) ? l.ts : 0;
    const d = new Date(ts * 1000).toLocaleTimeString('fr-FR');
    return `[${d}] [${String(l.level || 'info').toUpperCase()}] ${l.msg || ''}`;
  });
  copyText(lines.join('\n'));
}
function clearVisibleLog() {
  const chrono = [...((state && state.log) || [])].reverse();
  if (!chrono.length) return;
  logClearWatermark = chrono.slice(-8).map(logSigOf);
  logClearTs = Number(chrono[chrono.length - 1].ts) || 0;
  _sigLog = '';
  renderLog();
}
function toggleLogExpand() {
  const lp = document.getElementById('logpanel');
  const expBtn = document.getElementById('logExpandBtn');
  const togBtn = document.getElementById('logToggleBtn');
  if (!lp) return;
  if (lp.classList.contains('min')) {
    lp.classList.remove('min');
    if (togBtn) togBtn.setAttribute('aria-expanded', 'true');
    try { localStorage.setItem('mcc.logmin', 'false'); } catch (e) {}
  }
  const exp = lp.classList.toggle('expanded');
  if (expBtn) {
    expBtn.classList.toggle('sel', exp);
    expBtn.setAttribute('aria-pressed', exp ? 'true' : 'false');
  }
  try { localStorage.setItem('mcc.logexpanded', String(exp)); } catch (e) {}
}

/* ===== Zone de détail ===== */
/* Tuiles cliquables : « serveurs UP » filtre l'explorateur sur les UP,
   « serveurs suivis » remet l'affichage complet, « tools actifs » applique le
   tri par nombre d'outils, « machines distantes » ouvre le dialog SSH. */
function tileFilter(mode) {
  const next = (mode === 'up' && expStateFilter !== 'up' && overviewFilter !== 'up') ? 'up' : 'all';
  setExpStateFilter(next);
}
function tileSortTools() {
  const sel = document.getElementById('sortSel');
  if (sel) sel.value = 'tools';
  applySort('tools');
}
function clearFilter() {
  const inp = document.getElementById('srvFilter');
  if (inp) inp.value = '';
  renderServers();
}
function renderOverview() {
  const s = state.stats;
  const a = state.access || {};
  const pendCount = (a.pending || 0) + (a.pairs || 0);
  const tls = (state.settings && state.settings.tls) || {};
  const sshHosts = (state.settings && state.settings.ssh_hosts) || [];
  const sessActive = state.sessions_active || 1;
  const srvs = state.servers || [];
  const lockedSrvs = srvs.filter(x => x.locked && !x.paired && x.state !== 'UP');
  const downSrvs = srvs.filter(x => x.enabled && (x.state === 'DOWN' || x.state === 'PORT-ONLY') && !(x.locked && !x.paired));
  const pausedSrvs = srvs.filter(x => !x.enabled || x.state === 'DISABLED');
  const startingSrvs = srvs.filter(x => x.enabled && (x.state === 'STARTING' || x.state === 'PROBING'));
  const totalLogs = (state.log || []).length;
  const recentLogs = (state.log || []).slice(0, 5);
  const sig = JSON.stringify([
    s, a, tls, sshHosts, sessActive, overviewFilter, expStateFilter,
    downSrvs.map(x => [x.id, x.label, x.state, x.can_start, x.last_error]),
    lockedSrvs.map(x => [x.id, x.label, x.state]),
    pausedSrvs.length, startingSrvs.length,
    unfollowedAnnouncements,
    recentLogs, totalLogs,
    srvs.length
  ]);
  const contentEl = document.getElementById('content');
  if (!selectedId && sig === _sigOverview && contentEl && contentEl.querySelector('.dash-head')) return;
  _sigOverview = sig;
  const tlsLabel = tls.active ? 'TLS 1.3 actif' : (tls.compiled ? 'Loopback HTTP (TLS 1.3 pr\u00eat)' : 'Loopback seul');
  let upTileCls = 'tile-ok';
  let upNumCls = 'ok';
  let upMeta = '100 % op\u00e9rationnels';
  if (s.total === 0) {
    upTileCls = 'tile-neutral';
    upNumCls = '';
    upMeta = 'Aucun serveur suivi';
  } else if (s.up === s.total) {
    upTileCls = 'tile-ok';
    upNumCls = 'ok';
    upMeta = '100 % op\u00e9rationnels';
  } else {
    const parts = [];
    if (downSrvs.length) parts.push(esc(downSrvs.length) + ' en panne');
    if (lockedSrvs.length) parts.push(esc(lockedSrvs.length) + ' verrouill\u00e9' + (lockedSrvs.length > 1 ? 's' : ''));
    if (pausedSrvs.length) parts.push(esc(pausedSrvs.length) + ' en pause');
    if (startingSrvs.length) parts.push(esc(startingSrvs.length) + ' en d\u00e9marrage');
    if (downSrvs.length > 0) {
      upTileCls = 'tile-warn';
      upNumCls = 'warn';
    } else if (s.up === 0) {
      upTileCls = 'tile-neutral';
      upNumCls = '';
    } else {
      upTileCls = 'tile-acc';
      upNumCls = '';
    }
    upMeta = parts.length ? parts.join(' \u00b7 ') : esc(s.total - s.up) + ' hors ligne';
  }
  const downHtml = downSrvs.length
    ? downSrvs.slice(0, 6).map(x => `<div class="dash-item">
        <button type="button" class="dash-link" data-act="select" data-id="${esc(x.id)}"><span class="nm">${esc(x.label)}</span><span class="meta">${esc(x.state)}${x.last_error ? ' &mdash; ' + esc(String(x.last_error).slice(0, 48)) : ''}</span></button>
        <button type="button" class="primary" data-act="row-act" data-op="${x.can_start ? 'start' : 'refresh'}" data-id="${esc(x.id)}">${ic(x.can_start ? 'play' : 'refresh')} ${x.can_start ? 'Relancer' : 'Sonder'}</button>
      </div>`).join('')
      + (downSrvs.length > 6 ? `<button type="button" class="dash-more" data-act="exp-state" data-mode="down">+${esc(downSrvs.length - 6)} autre${downSrvs.length - 6 > 1 ? 's' : ''} &mdash; voir dans l&rsquo;explorateur</button>` : '')
    : '<div class="empty-note">Tous les serveurs actifs r&eacute;pondent normalement.</div>';
  const lockedHtml = lockedSrvs.length
    ? lockedSrvs.slice(0, 6).map(x => `<div class="dash-item">
        <button type="button" class="dash-link" data-act="select" data-id="${esc(x.id)}"><span class="nm">${esc(x.label)}</span><span class="meta">Signature RFC 9421 requise</span></button>
        <button type="button" class="primary" data-act="server-pair" data-id="${esc(x.id)}">${ic('lock')} Appairer</button>
      </div>`).join('')
      + (lockedSrvs.length > 6 ? `<button type="button" class="dash-more" data-act="exp-state" data-mode="locked">+${esc(lockedSrvs.length - 6)} autre${lockedSrvs.length - 6 > 1 ? 's' : ''} &mdash; voir dans l&rsquo;explorateur</button>` : '')
    : '<div class="empty-note">Aucun serveur verrouill&eacute; en attente d&rsquo;appairage.</div>';
  const annHtml = unfollowedAnnouncements.length
    ? unfollowedAnnouncements.slice(0, 6).map(x => `<div class="dash-item">
        <div><div class="nm">${esc(x.name)}</div><div class="meta">127.0.0.1:${esc(x.port || '?')}${x.version ? ' &middot; v' + esc(x.version) : ''}</div></div>
        ${x.port ? `<button type="button" class="primary" data-act="follow-ann" data-name="${esc(x.name)}" data-port="${esc(x.port)}">${ic('check')} Suivre</button>` : ''}
      </div>`).join('')
    : '<div class="empty-note">Aucune annonce LiteMCP locale non suivie.</div>';
  const logHtml = recentLogs.length
    ? recentLogs.map(l => {
        const ts = Number.isFinite(l.ts) ? l.ts : 0;
        const d = new Date(ts * 1000).toLocaleTimeString('fr-FR');
        return `<div class="logline ${esc(l.level)}"><span class="t">${esc(d)}</span> ${esc(l.msg)}</div>`;
      }).join('')
    : '<div class="empty-note">Aucun &eacute;v&eacute;nement r&eacute;cent.</div>';
  const firstStartGuide = !srvs.length ? `
    <div class="welcome" id="welcome">
      <div class="welcome-mark">${ic('zap')}</div>
      <h2 class="big">Bienvenue dans MCP Control</h2>
      <div class="hint">Aucun serveur MCP n'est encore configur&eacute;. Lancez un scan local, explorez la boutique ou ajoutez une machine distante :</div>
      <div class="qa">
        <button type="button" class="primary" data-act="scan">${ic('search')} Scanner les ports locaux</button>
        <button type="button" data-act="store">${ic('cart')} Ouvrir la Boutique</button>
        <button type="button" data-act="scan-net">${ic('globe')} Scanner le r&eacute;seau</button>
        <button type="button" data-act="machines">${ic('monitor')} Choisir une machine</button>
      </div>
      <div class="hint hint-gap">Le scan utilise la plage configur&eacute;e. Pour une machine du LAN : connexion SSH par cl&eacute;,
      l'app liste ses ports en &eacute;coute, y sonde les serveurs MCP et les importe ici.</div>
    </div>` : '';
  const allActive = !overviewFilter && expStateFilter === 'all';
  const upActive = overviewFilter === 'up' || expStateFilter === 'up';
  contentEl.innerHTML = `
    <div class="dash-head">
      <div>
        <h1>Tableau de bord MCP Control</h1>
        <div class="sub">Supervision des serveurs MCP, passerelle sign&eacute;e Ed25519 et centre de s&eacute;curit&eacute; local</div>
      </div>
      <div class="d-actions">
        <button type="button" data-act="tile" data-mode="all" aria-pressed="${allActive ? 'true' : 'false'}" class="${allActive ? 'primary' : ''}">Tous les serveurs (${esc(s.total)})</button>
        <button type="button" data-act="machines">${ic('monitor')} Machines SSH (${esc(sshHosts.length)})</button>
      </div>
    </div>
    <div class="tiles">
      <button type="button" class="tile ${upTileCls}${upActive ? ' sel' : ''}" data-act="tile" data-mode="up" aria-pressed="${upActive ? 'true' : 'false'}"
        title="Cliquer : ne montrer que les serveurs UP dans l'explorateur (re-cliquer pour tout afficher)">
        <div class="tile-top"><div class="n ${upNumCls}">${esc(s.up)} / ${esc(s.total)}</div><span class="tile-ic">${ic('activity')}</span></div>
        <div class="lbl">Serveurs UP / Total</div>
        <div class="meta">${upMeta}</div>
      </button>
      <button type="button" class="tile tile-acc" data-act="tile-tools"
        title="Cliquer : trier l'explorateur par nombre d'outils expos&eacute;s">
        <div class="tile-top"><div class="n">${esc(s.tools)}</div><span class="tile-ic">${ic('terminal')}</span></div>
        <div class="lbl">Outils expos&eacute;s</div>
        <div class="meta">Passerelle POST /mcp</div>
      </button>
      <button type="button" class="tile ${pendCount > 0 ? 'tile-warn' : 'tile-ok'}" data-act="access-open" data-pending="1"
        title="Cliquer : ouvrir le centre Acc&egrave;s &amp; S&eacute;curit&eacute;">
        <div class="tile-top"><div class="n ${pendCount > 0 ? 'warn' : 'ok'}">${esc(pendCount)}</div><span class="tile-ic">${ic('shield')}</span></div>
        <div class="lbl">Confirmations &amp; Appairages</div>
        <div class="meta">${esc(a.pending || 0)} ex&eacute;cution(s) &middot; ${esc(a.pairs || 0)} appairage(s)</div>
      </button>
      <button type="button" class="tile tile-acc" data-act="access-open"
        title="Cliquer : inspecter l'identit&eacute; serveur, les sessions locales et TLS 1.3">
        <div class="tile-top"><div class="n">${esc(sessActive)}</div><span class="tile-ic">${ic('lock')}</span></div>
        <div class="lbl">Sessions locales &amp; TLS</div>
        <div class="meta">${esc(tlsLabel)}</div>
      </button>
    </div>
    ${firstStartGuide}
    <div class="dash-grid">
      <div class="dash-card">
        <h2 class="dash-card-h"><span>${ic('warn')} Serveurs en panne</span><span class="dash-cnt">${esc(downSrvs.length)}</span></h2>
        ${downHtml}
      </div>
      <div class="dash-card">
        <h2 class="dash-card-h"><span>${ic('lock')} Serveurs verrouill&eacute;s</span><span class="dash-cnt">${esc(lockedSrvs.length)}</span></h2>
        ${lockedHtml}
      </div>
      <div class="dash-card">
        <h2 class="dash-card-h"><span>${ic('activity')} D&eacute;tect&eacute;s, non suivis</span><span class="dash-cnt">${esc(unfollowedAnnouncements.length)}</span></h2>
        ${annHtml}
      </div>
      <div class="dash-card">
        <h2 class="dash-card-h"><span>${ic('terminal')} Derniers &eacute;v&eacute;nements</span><span class="dash-cnt">${esc(totalLogs)}</span></h2>
        ${logHtml}
      </div>
    </div>`;
}

/* ===== Onglet Conf agent : prêt à coller dans Hermes / Claude / etc. ===== */
const srvName = s => (s.label || s.id || 'serveur').toLowerCase().replace(/[^a-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '') || 'serveur';
/* URL http d'un transport : un hôte IPv6 (il contient « : ») doit être entouré
   de crochets — http://[::1]:4104/mcp et non http://::1:4104/mcp (invalide,
   ambigu). Un seul helper pour tous les blocs d'URL (audit 04 #5). */
const hostForUrl = h => { const s = String(h || ''); return s.includes(':') ? '[' + s + ']' : s; };
const srvUrl = s => {
  const h = s.transport_info.host || '127.0.0.1';
  return 'http://' + hostForUrl(h) + ':' + s.transport_info.port + (s.transport_info.path || '/mcp');
};
function hermesYaml(s) {
  const hUrl = hybridUrl(s);
  const n = srvName(s);
  const bCmd = (state && state.bridge_command) || 'mcp-control';
  if (hUrl) {
    const hArgs = ['bridge', '--url', hUrl].map(a => JSON.stringify(a)).join(', ');
    return 'mcp_servers:\n  ' + n + ':\n    command: ' + JSON.stringify(bCmd) + '\n    args: [' + hArgs + ']\n    enabled: true';
  }
  if (s.transport === 'http')
    return 'mcp_servers:\n  ' + n + ':\n    url: ' + srvUrl(s) + '\n    enabled: true';
  const args = (s.transport_info.args || []).map(a => JSON.stringify(a)).join(', ');
  return 'mcp_servers:\n  ' + n + ':\n    command: ' + (s.transport_info.program || 'npx') + '\n    args: [' + args + ']\n    enabled: true';
}
function mcpJson(s) {
  const hUrl = hybridUrl(s);
  const bCmd = (state && state.bridge_command) || 'mcp-control';
  if (hUrl) {
    return JSON.stringify({
      mcpServers: {
        [s.id]: {
          command: bCmd,
          args: ['bridge', '--url', hUrl],
        },
      },
    }, null, 2);
  }
  const n = srvName(s);
  const blk = s.transport === 'http'
    ? {type: 'http', url: srvUrl(s)}
    : {command: s.transport_info.program || 'npx', args: s.transport_info.args || []};
  return JSON.stringify({mcpServers: {[n]: blk}}, null, 2);
}
const quoteShellArg = a => {
  const str = String(a ?? '');
  return /[\s"'\\$`]/.test(str) ? '"' + str.replace(/(["\\$`])/g, '\\$1') + '"' : str;
};
function claudeCmd(s) {
  const hUrl = hybridUrl(s);
  const bCmd = (state && state.bridge_command) || 'mcp-control';
  if (hUrl)
    return 'claude mcp add ' + quoteShellArg(s.id) + ' -- ' + quoteShellArg(bCmd) + ' bridge --url ' + quoteShellArg(hUrl);
  const n = srvName(s);
  if (s.transport === 'http') return 'claude mcp add --transport http ' + n + ' ' + srvUrl(s);
  const prog = quoteShellArg(s.transport_info.program || 'npx');
  const args = (s.transport_info.args || []).map(quoteShellArg);
  return 'claude mcp add ' + n + ' -- ' + prog + (args.length ? ' ' + args.join(' ') : '');
}
function copyConf(btn) {
  const pre = btn.closest('.conf-block').querySelector('pre');
  copyText(pre.textContent); // repli execCommand intégré (audit 04 #11)
  btn.classList.add('copied');
  btn.innerHTML = ic('check') + '<span>Copi&eacute;</span>';
  setTimeout(() => {
    btn.classList.remove('copied');
    btn.innerHTML = ic('copy') + '<span>Copier</span>';
  }, 1500);
}
function hlYaml(src) {
  return String(src || '').split('\n').map(line => {
    const m = line.match(/^(\s*)([A-Za-z0-9_.-]+)(:)(\s*)(.*)$/);
    if (!m) return esc(line);
    const [, ind, key, col, sp, val] = m;
    let vHtml = esc(val);
    if (/^(true|false|null|\d+)$/.test(val)) vHtml = '<span class="tok-b">' + esc(val) + '</span>';
    else if (val) vHtml = '<span class="tok-s">' + esc(val) + '</span>';
    return esc(ind) + '<span class="tok-k">' + esc(key) + '</span><span class="tok-p">' + esc(col) + '</span>' + esc(sp) + vHtml;
  }).join('\n');
}
function hlJson(src) {
  const raw = String(src ?? '').replace(/\r\n?/g, '\n');
  const re = /("(?:\\.|[^"\\\r\n])*")(\s*:)?|\b(true|false|null)\b|(?<![A-Za-z0-9_])(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)(?![A-Za-z0-9_])/g;
  let out = '';
  let last = 0;
  let m;
  while ((m = re.exec(raw)) !== null) {
    if (m.index > last) out += escPre(raw.slice(last, m.index));
    if (m[1] !== undefined) {
      if (m[2] !== undefined) {
        out += '<span class="tok-k">' + escPre(m[1]) + '</span><span class="tok-p">' + escPre(m[2]) + '</span>';
      } else {
        out += '<span class="tok-s">' + escPre(m[1]) + '</span>';
      }
    } else if (m[3] !== undefined) {
      out += '<span class="tok-b">' + escPre(m[3]) + '</span>';
    } else if (m[4] !== undefined) {
      out += '<span class="tok-b">' + escPre(m[4]) + '</span>';
    }
    last = re.lastIndex;
  }
  if (last < raw.length) out += escPre(raw.slice(last));
  return out;
}
function hlCli(src) {
  return String(src || '').split(' ').map((p, idx) => {
    if (idx === 0) return '<span class="tok-c">' + esc(p) + '</span>';
    if (p.startsWith('-')) return '<span class="tok-b">' + esc(p) + '</span>';
    if (/^https?:\/\//.test(p)) return '<span class="tok-s">' + esc(p) + '</span>';
    return esc(p);
  }).join(' ');
}
function confAgent(s) {
  const blk = (t, lang, html) =>
    '<div class="conf-block"><div class="conf-head"><span class="conf-lang">' + esc(lang) + '</span><span>' + t +
    '</span><button type="button" class="copy-btn" data-act="copy-conf">' + ic('copy') + '<span>Copier</span></button></div><pre>' + html + '</pre></div>';
  const hUrl = hybridUrl(s);
  if (hUrl) {
    const lockMsg = s.via_ssh
      ? '<b>Mode hybride SSH impos&eacute; :</b> ce serveur n\u2019est joignable que via SSH depuis MCP Control. Tous les blocs ci-dessous passent exclusivement par le pont sign&eacute; <code>mcp-control bridge</code>.'
      : '<b>Mode hybride impos&eacute; :</b> la configuration <code>stdio</code> directe est bloqu&eacute;e (elle contournerait l\'authentification Ed25519, les droits client, le filtrage des outils et le journal d\'audit). Tous les blocs ci-dessous passent exclusivement par le pont sign&eacute; <code>mcp-control bridge</code>.';
    return '<h2 class="sec">Connecter ce serveur &agrave; un agent</h2>' +
      '<div class="conf-note conf-hybrid-lock">' + ic('lock') + '<span>' + lockMsg + '</span></div>' +
      blk('JSON mcpServers (Claude, Cursor, VS Code&hellip;) &mdash; Pont hybride Ed25519 (' + esc(hUrl) + ')', 'JSON', hlJson(mcpJson(s))) +
      blk('Hermes &mdash; config.yaml (Pont hybride Ed25519)', 'YAML', hlYaml(hermesYaml(s))) +
      blk('claude mcp add (Pont hybride Ed25519)', 'CLI', hlCli(claudeCmd(s)));
  }
  return '<h2 class="sec">Connecter ce serveur &agrave; un agent</h2>' +
    '<div class="conf-note">Copie le bloc de ton agent &mdash; Hermes : colle sous <b>mcp_servers:</b> dans <b>~/.hermes/config.yaml</b>. Cl&eacute;s API : bloc <b>env:</b> (Hermes) ou <b>"env"</b> (JSON) &mdash; voir la description du serveur.</div>' +
    blk('Hermes &mdash; config.yaml', 'YAML', hlYaml(hermesYaml(s))) +
    blk('JSON mcpServers (Claude, Cursor, VS Code&hellip;)', 'JSON', hlJson(mcpJson(s))) +
    blk('claude mcp add', 'CLI', hlCli(claudeCmd(s)));
}
let toolSearchQuery = '';
const expandedTools = new Set();

function mdInline(t) {
  return esc(t)
    .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
    .replace(/(^|[^*])\*([^*\s][^*]*)\*/g, '$1<i>$2</i>')
    .replace(/`([^`]+)`/g, '<code>$1</code>');
}
const toolArgsDraft = new Map();
const toolTestState = new Map();

function buildArgsTemplate(schema) {
  if (!schema || typeof schema !== 'object' || !schema.properties || typeof schema.properties !== 'object') {
    return {};
  }
  const out = {};
  for (const [k, def] of Object.entries(schema.properties)) {
    if (!def || typeof def !== 'object') { out[k] = ''; continue; }
    if (Object.prototype.hasOwnProperty.call(def, 'default')) { out[k] = def.default; continue; }
    if (Array.isArray(def.enum) && def.enum.length) { out[k] = def.enum[0]; continue; }
    const ty = Array.isArray(def.type) ? def.type[0] : def.type;
    if (ty === 'number' || ty === 'integer') out[k] = 0;
    else if (ty === 'boolean') out[k] = false;
    else if (ty === 'array') out[k] = [];
    else if (ty === 'object') out[k] = {};
    else out[k] = '';
  }
  return out;
}
function formatSchemaType(def) {
  if (!def || typeof def !== 'object') return 'any';
  if (Array.isArray(def.type)) return def.type.join(' | ');
  if (def.type === 'array' && def.items && def.items.type) return 'array<' + def.items.type + '>';
  if (def.type) return String(def.type);
  if (Array.isArray(def.enum)) return 'enum';
  if (Array.isArray(def.anyOf)) return def.anyOf.map(formatSchemaType).join(' | ');
  return 'any';
}
function renderToolParamsHtml(schema) {
  const props = schema && typeof schema === 'object' && schema.properties && typeof schema.properties === 'object'
    ? Object.entries(schema.properties) : [];
  if (!props.length) {
    return '<div class="empty-note">Aucun param&egrave;tre d&eacute;clar&eacute; dans le sch&eacute;ma (objet vide <code>{}</code>).</div>';
  }
  const req = new Set(Array.isArray(schema.required) ? schema.required : []);
  const rows = props.map(([name, def]) => {
    const isReq = req.has(name);
    const ty = formatSchemaType(def);
    const dHtml = def && def.description ? mdInline(def.description) : '<span class="t3">&mdash;</span>';
    const chips = [];
    if (def && Array.isArray(def.enum) && def.enum.length) {
      chips.push('<span class="param-chip">enum: ' + esc(def.enum.map(String).join(' | ')) + '</span>');
    }
    if (def && Object.prototype.hasOwnProperty.call(def, 'default')) {
      chips.push('<span class="param-chip">d&eacute;faut: ' + esc(JSON.stringify(def.default)) + '</span>');
    }
    return `<tr>
      <td><code>${esc(name)}</code>${isReq ? '<span class="param-req">requis</span>' : ''}</td>
      <td class="param-type">${esc(ty)}</td>
      <td>${dHtml}${chips.length ? '<div class="param-meta">' + chips.join('') + '</div>' : ''}</td>
    </tr>`;
  }).join('');
  return `<table class="tool-params"><thead><tr><th>Param&egrave;tre</th><th>Type</th><th>Description</th></tr></thead><tbody>${rows}</tbody></table>`;
}
function captureFocusIn(box) {
  const a = document.activeElement;
  if (!box || !a || !box.contains(a)) return null;
  if (a.id) return {id: a.id};
  const toolEl = a.closest ? a.closest('.tool[data-tool]') : null;
  if (toolEl && a.dataset && a.dataset.act) {
    return {tool: toolEl.dataset.tool, act: a.dataset.act};
  }
  if (a.dataset && a.dataset.act) {
    return {act: a.dataset.act, op: a.dataset.op || '', id: a.dataset.id || ''};
  }
  return null;
}
function restoreFocusIn(box, spec) {
  if (!box || !spec) return;
  let target = null;
  if (spec.id && !spec.act) {
    target = document.getElementById(spec.id);
  } else if (spec.tool) {
    const toolEl = [...box.querySelectorAll('.tool[data-tool]')].find(el => el.dataset.tool === spec.tool);
    if (toolEl) {
      target = [...toolEl.querySelectorAll('[data-act]')].find(el => el.dataset.act === spec.act)
        || toolEl.querySelector('.tool-head-btn');
    }
  } else if (spec.act) {
    const els = [...box.querySelectorAll('[data-act]')].filter(el => el.dataset.act === spec.act);
    target = els.find(el => (!spec.op || el.dataset.op === spec.op) && (!spec.id || el.dataset.id === spec.id))
      || els[0] || null;
  }
  if (target && !target.disabled && document.activeElement !== target) {
    try { target.focus({preventScroll: true}); } catch (e) {}
  }
}
function renderToolPanelHtml(s, t, expKey, panelId) {
  const schema = t.input_schema || null;
  const schemaJson = schema ? JSON.stringify(schema, null, 2) : null;
  const canRun = s.state === 'UP' && (s.transport === 'http' || s.transport === 'stdio') && (!s.locked || s.paired) && !t.disabled;
  const draft = toolArgsDraft.has(expKey)
    ? toolArgsDraft.get(expKey)
    : JSON.stringify(buildArgsTemplate(schema), null, 2);
  const st = toolTestState.get(expKey) || null;
  let resHtml = '';
  if (st) {
    if (st.loading) {
      resHtml = `<div class="tool-res" aria-busy="true"><div class="tool-res-head"><span class="spin">${ic('refresh')}</span><span>Ex&eacute;cution en cours&hellip;</span></div></div>`;
    } else if (st.error) {
      resHtml = `<div class="tool-res"><div class="tool-res-head"><span class="tool-res-err">Erreur</span><button type="button" class="copy-btn" data-act="copy-tool-res" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">${ic('copy')}<span>Copier le r&eacute;sultat</span></button></div><pre>${escPre(st.error)}</pre></div>`;
    } else if (st.result !== undefined) {
      const isErr = !!(st.result && st.result.isError);
      const badge = isErr ? '<span class="tool-res-err">isError: true</span>' : '<span class="tool-res-ok">OK</span>';
      const ms = st.elapsed_ms != null ? `<span>${esc(String(st.elapsed_ms))} ms</span>` : '';
      const raw = JSON.stringify(st.result, null, 2);
      resHtml = `<div class="tool-res"><div class="tool-res-head">${badge}${ms}<button type="button" class="copy-btn" data-act="copy-tool-res" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">${ic('copy')}<span>Copier le r&eacute;sultat</span></button></div><pre>${hlJson(raw)}</pre></div>`;
    }
  }
  const whyDisabled = t.disabled ? 'Outil d&eacute;sactiv&eacute; &mdash; r&eacute;active-le pour le tester.'
    : (s.locked && !s.paired) ? 'Serveur verrouill&eacute; : appaire-le d\u2019abord pour tester ses outils.'
    : s.state !== 'UP' ? 'Serveur hors ligne (UP requis pour ex&eacute;cuter un outil).'
    : 'Transport non support&eacute; pour l\u2019ex&eacute;cution directe.';
  const testerHtml = canRun ? `
    <div class="tool-sec-h"><span>Tester l'outil en direct</span></div>
    <textarea class="tool-args-ta" rows="5" spellcheck="false" aria-label="Arguments JSON pour ${esc(t.name)}"
      data-act="tool-args-input" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">${escPre(draft)}</textarea>
    <div class="tool-actions-bar">
      <button type="button" class="primary" data-act="tool-run" data-id="${esc(s.id)}" data-tool="${esc(t.name)}" ${st && st.loading ? 'disabled' : ''}>${st && st.loading ? `<span class="spin">${ic('refresh')}</span> Ex&eacute;cution&hellip;` : `${ic('play')} Ex&eacute;cuter`}</button>
      <button type="button" data-act="tool-args-format" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">Formater</button>
      <button type="button" data-act="tool-args-reset" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">R&eacute;initialiser</button>
    </div>
    ${resHtml}` : `<div class="empty-note">${whyDisabled}</div>`;
  return `<div id="${esc(panelId || '')}" class="tool-panel">
    <div class="tool-full-desc">${t.desc ? mdInline(t.desc) : '<span class="t3">Aucune description fournie par le serveur.</span>'}</div>
    <div class="tool-sec-h"><span>Param&egrave;tres</span></div>
    ${renderToolParamsHtml(schema)}
    <div class="conf-block">
      <div class="conf-head">
        <span class="conf-lang">JSON Schema</span>
        <span>inputSchema</span>
        <button type="button" class="copy-btn" data-act="copy-tool-rpc" data-tool="${esc(t.name)}">${ic('copy')}<span>Copier tools/call</span></button>
        <button type="button" class="copy-btn" data-act="copy-tool-schema" data-tool="${esc(t.name)}">${ic('copy')}<span>Copier sch&eacute;ma</span></button>
      </div>
      <pre>${schemaJson ? hlJson(schemaJson) : '<span class="tok-p">// Aucun inputSchema retourn&eacute; par ce serveur — clique sur « Sonder » pour rafra&icirc;chir</span>'}</pre>
    </div>
    ${testerHtml}
  </div>`;
}
function renderToolsRowsHtml(s) {
  const all = s.tools_list || [];
  const q = toolSearchQuery.trim().toLowerCase();
  const filtered = q ? all.filter(t => (t.name + ' ' + (t.desc || '')).toLowerCase().includes(q)) : all;
  if (!filtered.length) {
    return `<div class="empty-note">${all.length ? 'aucun outil ne correspond au filtre' : 'aucun tool list&eacute;'}</div>`;
  }
  return filtered.map((t, idx) => {
    const expKey = s.id + '::' + t.name;
    const isExp = expandedTools.has(expKey);
    const isToolSel = selectedTools.has(t.name);
    const panelId = 'toolPanel_' + idx;
    const propsCount = t.input_schema && t.input_schema.properties && typeof t.input_schema.properties === 'object'
      ? Object.keys(t.input_schema.properties).length : 0;
    const pBadge = propsCount > 0 ? `<span class="tool-pbadge">${esc(propsCount)} param.</span>` : '';
    return `<div class="tool ${t.disabled?'off':''} ${isExp?'expanded':''} ${isToolSel?'multi-sel':''}" data-tool="${esc(t.name)}"${isToolSel ? ' aria-selected="true"' : ''}>
       <div class="tool-head">
         <button type="button" class="tool-head-btn" data-act="tool-expand" data-tool="${esc(t.name)}" aria-expanded="${isExp ? 'true' : 'false'}" aria-controls="${esc(panelId)}" title="Cliquer pour inspecter · Ctrl+clic ou Shift+clic pour multi-s\u00e9lectionner">
           <span class="tool-chev" aria-hidden="true">&#9656;</span>
           <code>${esc(t.name)}</code>
           <span class="desc">${t.desc ? mdInline(t.desc) : '&mdash;'}</span>
           ${pBadge}
         </button>
         <button type="button" class="tgl ${t.disabled?'off':'on'}" role="switch" aria-checked="${t.disabled?'false':'true'}" aria-label="${t.disabled?'R&eacute;activer':'D&eacute;sactiver'} l'outil ${esc(t.name)}" title="${t.disabled?'R&eacute;activer ce tool':'D&eacute;sactiver ce tool'}"
                 data-act="tool-toggle" data-id="${esc(s.id)}" data-tool="${esc(t.name)}">
           ${t.disabled ? 'OFF' : 'ON'}
         </button>
       </div>
       ${isExp ? renderToolPanelHtml(s, t, expKey, panelId) : ''}
     </div>`;
  }).join('');
}
function updateToolSearchMeta() {
  const clr = document.getElementById('toolSearchClear');
  if (clr) setHidden(clr, !toolSearchQuery);
  const cnt = document.getElementById('toolSearchCount');
  if (cnt && detailCache) {
    const all = detailCache.tools_list || [];
    const q = toolSearchQuery.trim().toLowerCase();
    if (!q) { setHidden(cnt, true); }
    else {
      const n = all.filter(t => (t.name + ' ' + (t.desc || '')).toLowerCase().includes(q)).length;
      setText(cnt, n + ' / ' + all.length);
      setHidden(cnt, false);
    }
  }
}
function refreshToolsBox() {
  const box = document.getElementById('toolsRowsBox');
  if (!box || !detailCache) return;
  const spec = captureFocusIn(box);
  box.innerHTML = renderToolsRowsHtml(detailCache);
  restoreFocusIn(box, spec);
  updateToolSearchMeta();
}
function setToolSearch(el) {
  toolSearchQuery = el ? el.value : '';
  refreshToolsBox();
}
function clearToolSearch() {
  toolSearchQuery = '';
  const inp = document.getElementById('toolSearch');
  if (inp) { inp.value = ''; inp.focus(); }
  refreshToolsBox();
}
function toggleToolExpand(el) {
  if (!detailCache || !el || !el.dataset.tool) return;
  const expKey = detailCache.id + '::' + el.dataset.tool;
  if (expandedTools.has(expKey)) expandedTools.delete(expKey);
  else expandedTools.add(expKey);
  refreshToolsBox();
}
function copyToolSchema(toolName) {
  if (!detailCache || !toolName) return;
  const t = (detailCache.tools_list || []).find(x => x.name === toolName);
  const txt = t && t.input_schema ? JSON.stringify(t.input_schema, null, 2) : '{}';
  copyText(txt);
  toast('Sch\u00e9ma JSON copi\u00e9', 'ok', 1800);
}
function copyToolRpc(toolName) {
  if (!detailCache || !toolName) return;
  const t = (detailCache.tools_list || []).find(x => x.name === toolName);
  const expKey = detailCache.id + '::' + toolName;
  let args = buildArgsTemplate(t && t.input_schema);
  if (toolArgsDraft.has(expKey)) {
    const raw = toolArgsDraft.get(expKey);
    try {
      args = raw.trim() ? JSON.parse(raw) : {};
    } catch (e) {
      toast('Brouillon JSON invalide : ' + e.message, 'err', 3500);
      return;
    }
  }
  const rpc = {jsonrpc: '2.0', id: 1, method: 'tools/call', params: {name: toolName, arguments: args}};
  copyText(JSON.stringify(rpc, null, 2));
  toast('Payload tools/call copi\u00e9', 'ok', 1800);
}
function copyToolResult(id, toolName) {
  if (!id || !toolName) return;
  const st = toolTestState.get(id + '::' + toolName);
  if (!st) return;
  const txt = st.error ? String(st.error) : (st.result !== undefined ? JSON.stringify(st.result, null, 2) : '');
  if (txt) copyText(txt);
}
function saveToolArgsDraft(el) {
  if (!el || !el.dataset.id || !el.dataset.tool) return;
  toolArgsDraft.set(el.dataset.id + '::' + el.dataset.tool, el.value);
}
function formatToolArgsDraft(id, toolName) {
  if (!detailCache || !id || !toolName) return;
  const expKey = id + '::' + toolName;
  const t = (detailCache.tools_list || []).find(x => x.name === toolName);
  const raw = toolArgsDraft.has(expKey)
    ? toolArgsDraft.get(expKey)
    : JSON.stringify(buildArgsTemplate(t && t.input_schema), null, 2);
  try {
    const parsed = raw.trim() ? JSON.parse(raw) : {};
    toolArgsDraft.set(expKey, JSON.stringify(parsed, null, 2));
    refreshToolsBox();
    toast('JSON format\u00e9', 'ok', 1200);
  } catch (e) {
    toast('JSON invalide : ' + e.message, 'err', 3500);
  }
}
function resetToolArgsDraft(id, toolName) {
  if (!detailCache || !id || !toolName) return;
  const expKey = id + '::' + toolName;
  const t = (detailCache.tools_list || []).find(x => x.name === toolName);
  toolArgsDraft.set(expKey, JSON.stringify(buildArgsTemplate(t && t.input_schema), null, 2));
  toolTestState.delete(expKey);
  refreshToolsBox();
}
async function runToolTest(id, toolName) {
  if (!id || !toolName) return;
  const expKey = id + '::' + toolName;
  const raw = toolArgsDraft.has(expKey)
    ? toolArgsDraft.get(expKey)
    : (() => {
        const t = detailCache && (detailCache.tools_list || []).find(x => x.name === toolName);
        return JSON.stringify(buildArgsTemplate(t && t.input_schema), null, 2);
      })();
  let parsed;
  try {
    parsed = raw.trim() ? JSON.parse(raw) : {};
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      throw new Error('Les arguments doivent \u00eatre un objet JSON { ... }.');
    }
  } catch (e) {
    toolTestState.set(expKey, {error: 'JSON invalide : ' + e.message});
    refreshToolsBox();
    return;
  }
  toolTestState.set(expKey, {loading: true});
  refreshToolsBox();
  const j = await postAction({action: 'call_tool', id, tool: toolName, arguments: parsed}, {quiet: true});
  if (j.ok) {
    toolTestState.set(expKey, {result: j.result, elapsed_ms: j.elapsed_ms});
  } else {
    toolTestState.set(expKey, {error: j.error || 'erreur lors de l\u2019appel'});
  }
  refreshToolsBox();
}
async function setAllTools(id, enableAll) {
  if (!detailCache || detailCache.id !== id) return;
  const list = (detailCache.tools_list || []).filter(t => enableAll ? t.disabled : !t.disabled);
  if (!list.length) return;
  const btns = [...document.querySelectorAll('.tools-bar button[data-act="tools-all-on"], .tools-bar button[data-act="tools-all-off"]')];
  btns.forEach(b => { b.disabled = true; });
  try {
    // UI-C02 : 1 seule action atomique côté moteur au lieu de N requêtes parallèles.
    await postAction({action: 'set_all_tools', id, enable: !!enableAll}, {quiet: true});
    const r = await fetch('/api/detail?id=' + encodeURIComponent(id));
    if (r.ok) { detailCache = await r.json(); renderDetail(); }
    await load();
  } finally {
    btns.forEach(b => { b.disabled = false; });
  }
}

function renderDetail() {
  const s = detailCache;
  if (!s || s.id !== selectedId) return;
  const aEl = document.activeElement;
  if (aEl && (aEl.id === 'scProgram' || aEl.id === 'scArgs' || aEl.id === 'scWorkdir' || aEl.id === 'toolSearch'
    || aEl.id === 'swAskPrompt' || aEl.id === 'swAskFrom' || aEl.id === 'swAskModel' || aEl.id === 'swCfgEditor'
    || (aEl.classList && aEl.classList.contains('tool-args-ta')))) return;
  const sig = JSON.stringify([s, detailTab, toolSearchQuery, Object.keys(FRAMEWORKS).length]);
  const contentEl = document.getElementById('content');
  if (selectedId && sig === _sigDetail && contentEl && contentEl.querySelector('.d-head')) return;
  _sigDetail = sig;
  const sameSrv = contentEl && contentEl.dataset.srvId === s.id;
  const typed = {};
  if (sameSrv) {
    ['scProgram', 'scArgs', 'scWorkdir'].forEach(idOld => {
      const el = document.getElementById(idOld);
      if (el && el.value) typed[idOld] = el.value;
    });
  }
  const focusSpec = captureFocusIn(contentEl);
  const ti = s.transport === 'http'
    ? `HTTP ${s.transport_info.host ? s.transport_info.host + ':' : ''}${s.transport_info.port}${s.transport_info.path || '/mcp'}`
    : `stdio \u00b7 ${s.transport_info.program} ${(s.transport_info.args || []).join(' ')}`;
  const kvRow = (k, v) => {
    const cpy = v && v !== '—';
    return `<div class="k">${k}</div><div class="v${cpy ? ' cpy' : ''}"${cpy
      ? ` role="button" tabindex="0" aria-label="Copier ${esc(k)} : ${esc(v)}" title="Cliquer pour copier dans le presse-papiers" data-act="copy-self"` : ''}>${esc(v)}</div>`;
  };
  const kv = [
    ['id', s.id], ['transport', ti],
    ['origine', s.origin],
    ['bind', s.bound_host ?? '—'], ['serveur MCP', s.server_name ?? '—'],
    ['version', s.version ?? '—'], ['protocole', s.protocol ?? '—'],
    ['framework', s.framework
      ? ((FRAMEWORKS[s.framework] || {label: s.framework, lang: '?'}).label
         + ' (' + (FRAMEWORKS[s.framework] || {lang: s.lang || '?'}).lang
         + ' · spec ' + (s.protocol || '?') + ')')
      : '—'],
    ...(s.in_container ? [['conteneur', s.container_name ? 'Docker (' + s.container_name + ')' : 'Docker']] : []),
    ['sonde', s.via_ssh ? 'via SSH (interne à la machine)' : 'directe'],
    ['latence', s.latency_ms != null ? s.latency_ms + ' ms' : '—'],
    ['dernier vu', s.last_seen ? new Date(s.last_seen * 1000).toLocaleString('fr-FR') : '—'],
    ['erreur', s.last_error ?? '—'],
  ].map(([k, v]) => kvRow(k, v)).join('');
  const starting = s.enabled && s.managed && s.state !== 'UP';
  const dispState = starting ? 'STARTING' : s.state;
  const etatProcess = s.state === 'UP' ? 'en marche'
    : starting ? 'au d\u00e9marrage\u2026'
    : s.state === 'PORT-ONLY' ? 'port occup\u00e9 (aucune r\u00e9ponse MCP)'
    : s.state === 'DISABLED' ? 'd\u00e9sactiv\u00e9' : 'arr\u00eat\u00e9';
  const cmd = s.start_cmd
    ? s.start_cmd.program + ' ' + (s.start_cmd.args || []).join(' ') + (s.start_cmd.workdir ? '  \u00b7  ' + s.start_cmd.workdir : '')
    : null;
  const kvd = [
    ['contr&ocirc;le', s.managed ? 'process lanc\u00e9 et maintenu par MCP Control'
      : (cmd ? 'commande pr\u00eate \u2014 process externe non d\u00e9marr\u00e9 par ici' : 'aucune \u2014 process externe')],
    ['&eacute;tat du process', etatProcess],
    ['pid', s.pid ?? '—'],
    ['auto-restart', s.auto_restart ? 'ON \u2014 relance automatique au boot/crash' : 'OFF'],
    ['commande', cmd ?? '—'],
  ].map(([k, v]) => kvRow(k, v)).join('');
  const needsPair = s.transport === 'http' && s.locked && !s.paired;
  const foldOpen = name => {
    try {
      const v = localStorage.getItem('mcc.fold.' + name);
      if (v !== null) return v === 'true';
    } catch (e) {}
    return name === 'demarrage' && !s.start_cmd && s.state !== 'UP' && !needsPair;
  };
  const fold = (name, title, inner) =>
    `<details class="fold" data-fold="${name}"${foldOpen(name) ? ' open' : ''}><summary>${title}</summary>${inner}</details>`;
  const nOn = (s.tools_list || []).filter(t => !t.disabled).length;
  const nTot = (s.tools_list || []).length;
  const toolsHtml = `<h2 class="sec">Outils &mdash; ${esc(nOn)} actif${nOn > 1 ? 's' : ''} sur ${esc(nTot)}${nTot - nOn ? ` (${esc(nTot - nOn)} d&eacute;sactiv&eacute;${nTot - nOn > 1 ? 's' : ''})` : ''}</h2>
    ${nTot > 0 ? `<div class="tools-bar">
      <div class="tools-search-wrap">
        <label for="toolSearch" class="sr-only">Rechercher un outil</label>
        <input id="toolSearch" placeholder="Filtrer les outils de ce serveur&hellip;" data-act="tool-search" value="${esc(toolSearchQuery)}">
        <button type="button" id="toolSearchClear" class="search-clear" ${toolSearchQuery ? '' : 'hidden'} data-act="tool-search-clear" aria-label="Effacer le filtre d'outils">&times;</button>
      </div>
      <span id="toolSearchCount" class="tool-search-count" ${toolSearchQuery ? '' : 'hidden'}></span>
      <button type="button" data-act="tools-all-on" data-id="${esc(s.id)}" ${nOn === nTot ? 'disabled' : ''}>${ic('check')} Tout activer</button>
      <button type="button" data-act="tools-all-off" data-id="${esc(s.id)}" ${nOn === 0 ? 'disabled' : ''}>${ic('x')} Tout couper</button>
    </div>` : ''}
    <div id="toolsRowsBox">${renderToolsRowsHtml(s)}</div>`;
  const endpointLabel = s.transport === 'http'
    ? ((s.transport_info.host || '127.0.0.1') + ':' + s.transport_info.port + (s.transport_info.path || '/mcp'))
    : (hybridUrl(s) || ('stdio · ' + (s.transport_info.program || '')));
  const showErrBanner = !!(s.last_error && !(needsPair && /401|server_pair|signature|verrouill/i.test(s.last_error)));
  contentEl.dataset.srvId = s.id;
  contentEl.innerHTML = `
    <nav class="crumb" aria-label="Fil d'Ariane"><button type="button" class="crumb-link" data-act="go-home">Tableau de bord</button><span class="crumb-sep" aria-hidden="true">/</span><b>${esc(s.label)}</b></nav>
    <div class="d-head st-${esc(dispState)}">
      <div class="d-head-top">
        <span class="dot dot-lg ${esc(dispState)}"></span>${srvStoreLogo(s, true)}
        <h1>${esc(s.label)}</h1>
        <span class="pill ${esc(dispState)}" ${starting ? 'title="Process lanc&eacute;, MCP pas encore r&eacute;pondu"' : ''}>${starting ? 'AU D&Eacute;MARRAGE' : esc(s.state)}</span>
        <button type="button" class="d-head-endpoint cpy" data-act="copy-text" data-text="${esc(endpointLabel)}" title="Cliquer pour copier : ${esc(endpointLabel)}">${ic('copy')}<span>${esc(endpointLabel)}</span></button>
      </div>
      <div class="d-meta-strip">
        ${langChip(s)}
        ${dockerBadge(s, true)}
        ${typeBadge(s)}
        ${swarmBadge(s)}
        ${frameworkBadge(s)}
        ${lockBadge(s, true)}
        ${s.transport === 'stdio' ? '<span class="tag hybrid" title="Pont hybride natif MCP Control : Streamable HTTP POST ' + esc(s.hybrid_path || '') + ' prot&eacute;g&eacute; par signature Ed25519 LiteMCP">HYBRIDE &middot; HTTP + AUTH</span>' : ''}
        ${s.via_ssh ? '<span class="tag hybrid" title="Pont hybride natif MCP Control : Streamable HTTP POST ' + esc(s.hybrid_path || '') + ' prot&eacute;g&eacute; par signature Ed25519 LiteMCP via SSH">HYBRIDE &middot; SSH + AUTH</span>' : ''}
        ${s.protocol ? `<span class="d-meta-pill">MCP ${esc(s.protocol)}</span>` : ''}
        ${s.server_name ? `<span class="d-meta-pill">${esc(s.server_name)}${s.version ? ' v' + esc(s.version) : ''}</span>` : ''}
        ${s.latency_ms != null ? `<span class="d-meta-pill">${esc(String(s.latency_ms))} ms</span>` : ''}
        ${s.pid ? `<span class="d-meta-pill">PID ${esc(String(s.pid))}</span>` : ''}
      </div>
      <div class="d-actions">
        <div class="d-act-grp">
          ${s.can_start && s.state !== 'UP' ? `<button type="button" class="primary" data-act="row-act" data-op="start" data-id="${esc(s.id)}">${ic('play')} D&eacute;marrer</button>` : ''}
          ${(s.can_start || s.via_ssh) && s.state === 'UP' ? `<button type="button" class="danger" data-act="row-act" data-op="kill" data-id="${esc(s.id)}">${ic('stop')} Arr&ecirc;ter</button>` : ''}
          <button type="button" data-act="row-act" data-op="refresh" data-id="${esc(s.id)}">${ic('activity')} Sonder</button>
        </div>
        <span class="d-act-sep" aria-hidden="true"></span>
        <div class="d-act-grp">
          ${s.can_start ? `<button type="button" class="autobtn ${s.auto_restart?'on':''}" role="switch" aria-checked="${s.auto_restart?'true':'false'}" data-act="set-auto" data-id="${esc(s.id)}" data-on="${s.auto_restart?'0':'1'}"
            title="Autoboot : relance automatique de ce serveur au d&eacute;marrage de MCP Control et apr&egrave;s un crash">${ic('zap')} Autoboot ${s.auto_restart?'ON':'OFF'}</button>` : ''}
          <button type="button" class="favbtn ${s.favorite?'on':''}" role="switch" aria-checked="${s.favorite?'true':'false'}" data-act="set-fav" data-id="${esc(s.id)}" data-on="${s.favorite?'0':'1'}"
            title="Favori : &eacute;pingle ce serveur en haut de sa section dans l'explorateur (persist&eacute;)">${ic('star')} Favori ${s.favorite?'ON':'OFF'}</button>
        </div>
        <div class="d-act-grp d-act-right">
          <button type="button" class="btn-ghost" data-act="hide-server" data-id="${esc(s.id)}" title="Masquer ce serveur de l'affichage (session seulement : il revient &agrave; la r&eacute;ouverture de l'app)">${ic('eyeOff')} Masquer</button>
          <button type="button" data-act="row-act" data-op="toggle" data-id="${esc(s.id)}" title="${s.enabled
            ? 'Mettre en pause : le process est arr&ecirc;t&eacute; et le serveur n&#39;est plus sond&eacute; (il reste dans la liste, r&eacute;activable d&#39;un clic)'
            : 'Reprendre la supervision : le serveur sera sond&eacute; et d&eacute;marr&eacute; si une commande est connue'}">${s.enabled ? ic('pause') + ' Pause' : ic('play') + ' Reprendre'}</button>
          ${s.origin !== 'seed'
            ? `<button type="button" class="danger" data-act="remove-srv" data-id="${esc(s.id)}">${ic('trash')} Supprimer</button>`
            : `<button type="button" class="danger" data-act="remove-seed" data-id="${esc(s.id)}" title="Supprime ce seed de la config : il ne sera pas r&eacute;inject&eacute; au red&eacute;marrage">${ic('trash')} Supprimer</button>`}
        </div>
      </div>
    </div>
    ${needsPair ? `<div class="d-lock-banner"><div>${ic('lock')} <b>Serveur verrouill&eacute; par signature Ed25519 (RFC 9421)</b> &mdash; associez l'identit&eacute; locale <code>mcc-server</code> pour autoriser la sonde et le proxy.</div><button type="button" class="primary" data-act="server-pair" data-id="${esc(s.id)}">${ic('lock')} Appairer ce serveur</button></div>` : ''}
    ${s.origin === 'seed' ? '<div class="seed-note">Serveur seed : <b>Supprimer</b> l&rsquo;efface m&ecirc;me apr&egrave;s red&eacute;marrage (consign&eacute; dans config.json) ; <b>Pause</b> l&rsquo;&eacute;teint sans le supprimer.</div>' : ''}
    ${showErrBanner ? `<div class="d-err">${ic('warn')} ${esc(s.last_error)}</div>` : ''}
    ${fold('demarrage', 'D&eacute;marrage', `<div class="kv">${kvd}</div>${startCmdForm(s)}`)}
    ${fold('infos', 'Informations', `<div class="kv">${kv}</div>`)}
    <div class="tabs" role="tablist" aria-label="Vues du serveur">
      ${s.swarm_kind === 'resident' ? `<button type="button" class="tab ${detailTab === 'swarm-chat' ? 'sel' : ''}" role="tab" id="dtab-swarm-chat" aria-selected="${detailTab === 'swarm-chat' ? 'true' : 'false'}" aria-controls="dpanel-main" tabindex="${detailTab === 'swarm-chat' ? '0' : '-1'}" data-act="detail-tab" data-tab="swarm-chat">Conversation</button>` : ''}
      <button type="button" class="tab ${(detailTab === 'outils' || (s.swarm_kind !== 'resident' && (detailTab === 'swarm-chat' || detailTab === 'swarm-cfg'))) ? 'sel' : ''}" role="tab" id="dtab-outils" aria-selected="${(detailTab === 'outils' || (s.swarm_kind !== 'resident' && (detailTab === 'swarm-chat' || detailTab === 'swarm-cfg'))) ? 'true' : 'false'}" aria-controls="dpanel-main" tabindex="${(detailTab === 'outils' || (s.swarm_kind !== 'resident' && (detailTab === 'swarm-chat' || detailTab === 'swarm-cfg'))) ? '0' : '-1'}" data-act="detail-tab" data-tab="outils">Outils (${esc(nTot)})</button>
      ${s.swarm_kind === 'resident' ? `<button type="button" class="tab ${detailTab === 'swarm-cfg' ? 'sel' : ''}" role="tab" id="dtab-swarm-cfg" aria-selected="${detailTab === 'swarm-cfg' ? 'true' : 'false'}" aria-controls="dpanel-main" tabindex="${detailTab === 'swarm-cfg' ? '0' : '-1'}" data-act="detail-tab" data-tab="swarm-cfg">Config agent</button>` : ''}
      <button type="button" class="tab ${detailTab === 'conf' ? 'sel' : ''}" role="tab" id="dtab-conf" aria-selected="${detailTab === 'conf' ? 'true' : 'false'}" aria-controls="dpanel-main" tabindex="${detailTab === 'conf' ? '0' : '-1'}" data-act="detail-tab" data-tab="conf">Conf MCP</button>
    </div>
    <div id="dpanel-main" role="tabpanel" aria-labelledby="dtab-${esc((s.swarm_kind !== 'resident' && (detailTab === 'swarm-chat' || detailTab === 'swarm-cfg')) ? 'outils' : detailTab)}">
      ${detailTab === 'conf' ? confAgent(s)
        : (detailTab === 'swarm-chat' && s.swarm_kind === 'resident' && typeof renderSwarmDetailChat === 'function' ? renderSwarmDetailChat(s)
        : (detailTab === 'swarm-cfg' && s.swarm_kind === 'resident' && typeof renderSwarmDetailCfg === 'function' ? renderSwarmDetailCfg(s)
        : toolsHtml))}
    </div>`;
  if (sameSrv) {
    for (const idOld in typed) {
      const el = document.getElementById(idOld);
      if (el && !el.value) el.value = typed[idOld];
    }
  }
  updateToolSearchMeta();
  restoreFocusIn(contentEl, focusSpec);
}

function goHome() {
  for (const id of ['storeDlg', 'settingsDlg', 'accessDlg', 'machineDlg', 'dlg']) {
    const d = document.getElementById(id);
    if (d && d.open) {
      if (id === 'accessDlg' && accessTimer) { clearInterval(accessTimer); accessTimer = null; }
      d.close();
    }
  }
  selectedId = null;
  activeView = 'dashboard';
  explorerTab = 'mcp';
  const bMcp = document.getElementById('explTabMcp');
  const bSw = document.getElementById('explTabSwarm');
  if (bMcp) { bMcp.classList.toggle('sel', true); bMcp.setAttribute('aria-selected', 'true'); }
  if (bSw) { bSw.classList.toggle('sel', false); bSw.setAttribute('aria-selected', 'false'); }
  if (typeof swarmStopPoll === 'function') swarmStopPoll();
  if (typeof updateSwarmTopbarActive === 'function') updateSwarmTopbarActive();
  detailCache = null;
  _lastSelectedSrvSig = '';
  _detailEtag = null;
  _detailEtagId = null;
  _sigDetail = '';
  _sigOverview = '';
  _sigServers = '';
  detailTab = 'outils';
  toolSearchQuery = '';
  overviewFilter = null;
  expStateFilter = 'all';
  const fInp = document.getElementById('srvFilter');
  if (fInp) fInp.value = '';
  document.querySelectorAll('#expStatePills .exp-pill').forEach(b => {
    const sel = b.dataset.mode === 'all';
    b.classList.toggle('sel', sel);
    b.setAttribute('aria-pressed', sel ? 'true' : 'false');
  });
  closeDiscoverMenu();
  ctxClose(false);
  const contentEl = document.getElementById('content');
  if (contentEl) {
    delete contentEl.dataset.srvId;
    contentEl.scrollTop = 0;
  }
  render();
}

function select(id) {
  if (!id) { goHome(); return; }
  activeView = 'dashboard';
  if (typeof swarmStopPoll === 'function') swarmStopPoll();
  if (typeof updateSwarmTopbarActive === 'function') updateSwarmTopbarActive();
  const switched = selectedId !== id;
  selectedId = id;
  detailCache = null;
  _lastSelectedSrvSig = '';
  _detailEtag = null;
  _detailEtagId = null;
  _sigDetail = '';
  _sigOverview = '';
  if (switched) {
    toolSearchQuery = '';
    selectedTools.clear();
    lastToolAnchor = null;
    detailTab = 'outils';
  }
  renderServers();
  const s = byId(id);
  const contentEl = document.getElementById('content');
  if (contentEl && s) {
    delete contentEl.dataset.srvId;
    contentEl.innerHTML = `<nav class="crumb" aria-label="Fil d'Ariane"><button type="button" class="crumb-link" data-act="go-home">Tableau de bord</button><span class="crumb-sep" aria-hidden="true">/</span><b>${esc(s.label)}</b></nav>
      <div class="loading-card" role="status" aria-live="polite"><span class="spin">${ic('refresh')}</span><span>Chargement des d&eacute;tails de <b>${esc(s.label)}</b>&hellip;</span></div>`;
  }
  load();
}

/* Formulaire « commande de lancement » : pour un serveur local découvert par
   scan (port connu, commande inconnue) — poser la commande fait apparaître les
   boutons ▶/■ et l'autoboot, comme un serveur ajouté à la main. */
function startCmdForm(s) {
  if (s.can_start || s.transport === 'stdio' || s.origin === 'seed') return '';
  const host = s.transport_info && s.transport_info.host;
  if (host && !isLoopbackHost(host) && !s.via_ssh) return '<div class="conf-note note-gap">Serveur distant : la commande de lancement appartient &agrave; sa machine (' + esc(host) + '), pas à MCP Control.</div>';
  const port = (s.transport_info || {}).port || 0;
  const intro = s.via_ssh
    ? 'Serveur distant via SSH (' + esc(host || '') + ') &mdash; aucune commande de lancement enregistr&eacute;e. Indique comment le lancer sur sa machine via SSH :'
    : 'Aucune commande de lancement connue : c\'est pourquoi les boutons <b>&#9654;/&#9632;</b> et <b>&#9889;</b> sont absents. Indique comment lancer ce serveur :';
  return `<div class="startcmd-form note-gap">
    <div class="conf-note">${intro}</div>
    <div class="machine-form">
      <input id="scProgram" aria-label="Programme de lancement" placeholder="programme (ex. node, python)">
      <input id="scArgs" aria-label="Arguments de lancement" placeholder="arguments (ex. server.js --port ${esc(String(port))})">
      <input id="scWorkdir" aria-label="Dossier de travail" placeholder="dossier de travail (optionnel)">
      <button type="button" class="primary" data-act="set-start-cmd" data-id="${esc(s.id)}">Enregistrer</button>
    </div>
  </div>`;
}
async function setStartCmd(id) {
  const program = (document.getElementById('scProgram').value || '').trim();
  const args = (document.getElementById('scArgs').value || '').split(/\s+/).filter(Boolean);
  const workdir = (document.getElementById('scWorkdir').value || '').trim();
  if (!program) { toast('Programme requis (ex : node, python, chemin .exe)', 'err', 3500); return; }
  const j = await postAction({action: 'set_start_cmd', id, program, args, workdir: workdir || null}, {quiet: true});
  if (!j.ok) toast(j.error || 'erreur', 'err', 4000);
  else {
    detailCache = null; _detailEtag = null; _lastSelectedSrvSig = '';
    await load();
  }
}
async function removeSrv(id) {
  const s = byId(id);
  const ok = await askConfirm({
    title: 'Supprimer ce serveur',
    msg: 'Supprimer \u00ab ' + (s ? (s.label || id) : id) + ' \u00bb de la liste ?',
    detail: 'Un serveur re-d\u00e9tect\u00e9 par un scan r\u00e9appara\u00eetra.',
    okLabel: 'Supprimer',
    danger: true,
  });
  if (!ok) return;
  selectedId = null; detailCache = null; _sigOverview = ''; _sigDetail = ''; _lastSelectedSrvSig = '';
  act('remove', id);
}
async function removeSeedForever(id) {
  const ok = await askConfirm({
    title: 'Retirer ce seed durablement',
    msg: 'Retirer ce seed DURABLEMENT ?',
    detail: 'Il ne sera pas r\u00e9inject\u00e9 au red\u00e9marrage de MCP Control.',
    okLabel: 'Retirer durablement',
    danger: true,
  });
  if (!ok) return;
  selectedId = null; detailCache = null; _sigOverview = ''; _sigDetail = ''; _lastSelectedSrvSig = '';
  postAction({action: 'remove', id, durable: true}).then(() => load());
}

/* ===== Actions ===== */
/* ---- Menu contextuel (clic droit sur une ligne de serveur) ---------------- */
let ctxId = null;
let ctxOpener = null;
/* Entrées du menu contextuel courant : les items ne portent qu'un index
   (data-i), la table ACT relit la fermeture dans ce tableau — aucun code
   n'est jamais construit depuis une valeur reçue. */
let ctxItems = [];
function ctxClose(restore) {
  const m = document.getElementById('ctxmenu');
  const opener = ctxOpener;
  if (m) { m.hidden = true; m.innerHTML = ''; }
  ctxId = null;
  ctxOpener = null;
  if (restore && isFocusableVisible(opener)) {
    try { opener.focus(); } catch (e) {}
  }
}
/* Toast sémantique : toast(msg) neutre, toast(msg,'ok') succès,
   toast(msg,'err') erreur (affichée plus longtemps — un échec se lit) ;
   `ms` surcharge la durée quand le message est long à lire. */
function toast(msg, kind, ms) {
  const t = document.getElementById('toast');
  if (!t) return;
  clearTimeout(toast._t);
  clearTimeout(toast._hideT);
  t.textContent = msg;
  t.classList.remove('ok', 'err', 'warn');
  if (kind) t.classList.add(kind);
  if (typeof t.showPopover === 'function') {
    try {
      if (t.matches(':popover-open')) t.hidePopover();
      t.showPopover();
    } catch (e) {}
  }
  t.classList.add('on');
  toast._t = setTimeout(() => {
    t.classList.remove('on');
    if (typeof t.hidePopover === 'function') {
      toast._hideT = setTimeout(() => {
        if (!t.classList.contains('on')) {
          try { if (t.matches(':popover-open')) t.hidePopover(); } catch (e) {}
        }
      }, 220);
    }
  }, ms || (kind === 'err' ? 3500 : 1600));
}
/* Détail de l'alerte réseau (clic sur la pastille de la barre du Journal) :
   le message complet n'encombre pas la barre, il s'affiche à la demande. */
const NET_WARN_DETAIL = 'Réseau local injoignable : les serveurs distants ne peuvent pas être sondés'
  + ' — VPN actif ? (Proton VPN : activer «\u00a0autoriser les connexions LAN\u00a0»)';
function showNetDetail() {
  toast(NET_WARN_DETAIL, 'err', 6000);
}
function copyText(txt) {
  const done = () => toast('copié', 'ok');
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(txt).then(done).catch(() => { copyFallback(txt); done(); });
  } else { copyFallback(txt); done(); }
}
function copyFallback(txt) {
  const ta = document.createElement('textarea');
  ta.value = txt; ta.className = 'copy-ta';
  document.body.appendChild(ta); ta.select();
  try { document.execCommand('copy'); } catch (e) {}
  document.body.removeChild(ta);
}
function normPath(p) { return String(p || '').replace(/\\/g, '/').replace(/\/+$/, ''); }
// Chemin absolu + relatif (au workdir) : programme du start_cmd, ou fichier de
// script passé en argument quand le programme est un lanceur nu (node, npx,
// python…). Aucun chemin inventé : si rien n'est résolvable, la valeur est vide.
function srvPaths(s) {
  const ti = s.transport_info || {}, sc = s.start_cmd || {};
  const wd = normPath(ti.workdir || sc.workdir || '');
  const prog = ti.program || sc.program || '';
  const args = ti.args || sc.args || [];
  let abs = '';
  if (s.transport === 'stdio') {
    if (prog && /[\\/]/.test(prog)) abs = prog;
    else {
      const fileArg = args.find(a => /\.(js|mjs|cjs|jsx|ts|py|exe|jar|rb|php|sh|bat|cmd)$/i.test(a));
      // à défaut, un chemin absolu passé en argument (ex. server-filesystem C:/Users/X)
      const dirArg = args.find(a => /^[A-Za-z]:[\\/]/.test(a) || /^\/(?![-])/.test(a));
      if (fileArg) abs = (wd ? wd + '/' : '') + String(fileArg).replace(/\\/g, '/').replace(/^\/+/, '');
      else if (dirArg) abs = dirArg;
    }
  } else if (prog) abs = prog;
  const a = normPath(abs);
  const rel = (a && wd && a.toLowerCase().startsWith(wd.toLowerCase() + '/')) ? a.slice(wd.length + 1) : '';
  return { abs: a, rel: rel, workdir: wd };
}
// URL d'un serveur HTTP — variante gardée (la constante srvUrl plus haut ne
// vérifie pas le transport et sert au bloc « Conf agent »).
function ctxUrl(s) {
  if (s.transport === 'stdio') return hybridUrl(s);
  if (s.transport !== 'http') return '';
  const ti = s.transport_info || {};
  return 'http://' + hostForUrl(ti.host || '127.0.0.1') + ':' + ti.port + (ti.path || '/mcp');
}
function srvCmd(s) {
  const sc = s.start_cmd || {};
  if (!sc.program) return '';
  return [sc.program].concat(sc.args || []).join(' ');
}
function srvInfoText(s) {
  const p = srvPaths(s);
  const fw = (typeof FRAMEWORKS === 'object' && FRAMEWORKS[s.framework]) ? FRAMEWORKS[s.framework].label : (s.framework || '—');
  const L = [
    'serveur      : ' + s.label,
    'id           : ' + s.id,
    'état         : ' + s.state + (s.enabled ? '' : ' (désactivé)') + (s.auto_restart ? ' · autoboot' : ''),
    'transport    : ' + s.transport + (s.ws ? ' (WS)' : ''),
    'url          : ' + (ctxUrl(s) || '—'),
    'commande     : ' + (srvCmd(s) || '—'),
    'framework    : ' + fw,
    'langage      : ' + (s.lang || '—'),
    'version      : ' + (s.version || '—') + ' · protocole ' + (s.protocol || '—'),
    'nom annoncé  : ' + (s.server_name || '—'),
    'outils       : ' + s.tools_count + '/' + s.tools_total + ((s.tools_disabled || []).length ? ' (' + s.tools_disabled.length + ' off)' : ''),
    'latence      : ' + (s.latency_ms != null ? s.latency_ms + ' ms' : '—'),
    'pid          : ' + (s.pid != null ? s.pid : '—')
  ];
  if (p.abs) L.push('chemin       : ' + p.abs);
  if (p.rel) L.push('chemin rel.  : ' + p.rel);
  if (p.workdir) L.push('workdir      : ' + p.workdir);
  if (s.last_error) L.push('dernière err : ' + s.last_error);
  return L.join('\n');
}
async function openInExplorer(path) {
  if (!path) return;
  const j = await postAction({action: 'open_path', path: path}, {quiet: true});
  toast(j.ok ? 'explorateur ouvert' : ('échec : ' + (j.error || 'inconnu')), j.ok ? 'ok' : 'err');
}
async function fetchServerDetailsBatch(ids) {
  const out = [];
  for (const sid of ids) {
    const base = (state && state.servers ? state.servers : []).find(x => x.id === sid);
    if (!base) continue;
    try {
      const r = await fetch('/api/detail?id=' + encodeURIComponent(sid));
      if (r.ok) {
        out.push(await r.json());
        continue;
      }
    } catch (e) {}
    out.push(base);
  }
  return out;
}
function formatDetailedServerInfo(s) {
  const lines = [srvInfoText(s)];
  const tools = Array.isArray(s.tools_list) ? s.tools_list : [];
  if (tools.length) {
    lines.push('outils (' + tools.length + ') :');
    tools.forEach(t => {
      const st = t.disabled ? ' [OFF]' : '';
      const d = t.desc ? ' — ' + String(t.desc).replace(/\s+/g, ' ').trim() : '';
      lines.push('  - ' + t.name + st + d);
    });
  }
  return lines.join('\n');
}
async function copyMultiServersInfo(ids, asJson) {
  const details = await fetchServerDetailsBatch(ids);
  if (!details.length) return;
  if (asJson) {
    copyText(JSON.stringify(details, null, 2));
    toast('Infos JSON de ' + details.length + ' serveurs copi\u00e9es', 'ok', 2200);
  } else {
    const txt = details.map(formatDetailedServerInfo).join('\n\n' + '='.repeat(48) + '\n\n');
    copyText(txt);
    toast('Infos compl\u00e8tes de ' + details.length + ' serveurs copi\u00e9es', 'ok', 2200);
  }
}
function copyMultiServersMcpConfig(srvs) {
  const mcpServers = {};
  srvs.forEach(s => {
    try {
      const parsed = JSON.parse(mcpJson(s));
      if (parsed && parsed.mcpServers) {
        Object.assign(mcpServers, parsed.mcpServers);
      }
    } catch (e) {}
  });
  copyText(JSON.stringify({mcpServers}, null, 2));
  toast('Config MCP combin\u00e9e (' + srvs.length + ' serveurs) copi\u00e9e', 'ok', 2200);
}
function multiRowCtx(ev, ids) {
  const allSrvs = (state && state.servers ? state.servers : []);
  const srvs = ids.map(id => allSrvs.find(x => x.id === id)).filter(Boolean);
  if (srvs.length <= 1) {
    rowCtxSingle(ev, srvs[0] ? srvs[0].id : ids[0]);
    return;
  }
  const n = srvs.length;
  const items = [];
  const add = (lbl, fn, dis) => items.push({lbl: lbl, fn: fn, dis: !!dis});
  items.push({lbl: ic('activity') + ' ' + esc(n) + ' serveurs s\u00e9lectionn\u00e9s', fn: () => {}, dis: true, hdr: true});
  add('—', null, false);
  add(ic('copy') + ' Copier toutes les infos (' + n + ' serveurs)', () => copyMultiServersInfo(srvs.map(s => s.id), false));
  add(ic('copy') + ' Copier toutes les infos en JSON (' + n + ')', () => copyMultiServersInfo(srvs.map(s => s.id), true));
  add(ic('terminal') + ' Copier la config MCP combin\u00e9e (JSON)', () => copyMultiServersMcpConfig(srvs));
  add(ic('link') + ' Copier les URLs / commandes', () => {
    const lines = srvs.map(s => (s.label || s.id) + ' : ' + (ctxUrl(s) || srvCmd(s) || s.id));
    copyText(lines.join('\n'));
    toast(n + ' endpoints copi\u00e9s', 'ok', 2000);
  });
  add('—', null, false);
  add(ic('refresh') + ' Sonder les ' + n + ' serveurs', async () => {
    for (const s of srvs) await postAction({action: 'refresh', id: s.id}, {quiet: true});
    await load();
    toast(n + ' serveurs sond\u00e9s', 'ok', 2000);
  });
  const startable = srvs.filter(s => s.can_start && s.state !== 'UP');
  if (startable.length) {
    add(ic('play') + ' D\u00e9marrer (' + startable.length + ')', async () => {
      for (const s of startable) await postAction({action: 'start', id: s.id}, {quiet: true});
      await load();
    });
  }
  const enabledSrvs = srvs.filter(s => s.enabled);
  if (enabledSrvs.length) {
    add(ic('pause') + ' Mettre en pause (' + enabledSrvs.length + ')', async () => {
      for (const s of enabledSrvs) await postAction({action: 'toggle', id: s.id}, {quiet: true});
      await load();
    });
  }
  const pausedSrvs = srvs.filter(s => !s.enabled);
  if (pausedSrvs.length) {
    add(ic('play') + ' Reprendre la supervision (' + pausedSrvs.length + ')', async () => {
      for (const s of pausedSrvs) await postAction({action: 'toggle', id: s.id}, {quiet: true});
      await load();
    });
  }
  add('—', null, false);
  add(ic('check') + ' Tout s\u00e9lectionner (affich\u00e9s)', () => {
    visibleServerIds().forEach(x => selectedServers.add(x));
    renderServers();
  });
  add(ic('x') + ' Effacer la s\u00e9lection', () => {
    selectedServers.clear();
    renderServers();
  });
  showMenu(ev, items);
}
function rowCtx(ev, id) {
  ev.preventDefault();
  ev.stopPropagation();
  if (selectedServers.size > 1 && selectedServers.has(id)) {
    const vis = visibleServerIds();
    const ordered = vis.filter(x => selectedServers.has(x));
    const rest = [...selectedServers].filter(x => !ordered.includes(x));
    multiRowCtx(ev, [...ordered, ...rest]);
    return;
  }
  rowCtxSingle(ev, id);
}
function rowCtxSingle(ev, id) {
  const s = (state && state.servers ? state.servers : []).find(x => x.id === id);
  if (!s) return;
  const p = srvPaths(s);
  const items = [];
  const add = (lbl, fn, dis) => items.push({lbl: lbl, fn: fn, dis: !!dis});
  if (selectedId !== s.id) {
    add(ic('activity') + ' Ouvrir le d\u00e9tail', () => select(s.id));
    add('—', null, false);
  }
  add(ic('copy') + ' Copier toutes les infos', () => copyText(srvInfoText(s)));
  add(ic('tag') + ' Copier le nom', () => copyText(s.label));
  if (ctxUrl(s)) add(ic('link') + " Copier l'URL", () => copyText(ctxUrl(s)));
  if (srvCmd(s)) add(ic('terminal') + ' Copier la commande', () => copyText(srvCmd(s)));
  add(ic('folder') + ' Copier le chemin', () => copyText(p.abs), !p.abs);
  add(ic('folder') + ' Copier le chemin relatif', () => copyText(p.rel), !p.rel);
  add(ic('external') + " Ouvrir dans l'explorateur", () => openInExplorer(p.abs || p.workdir), !(p.abs || p.workdir));
  add('—', null, false);
  if (s.can_start) {
    add(s.state === 'UP' ? ic('stop') + ' Arrêter ce serveur' : ic('play') + ' Démarrer ce serveur',
        () => act(s.state === 'UP' ? 'kill' : 'start', s.id));
  }
  add(ic('refresh') + ' Sonder maintenant', () => act('refresh', s.id));
  add(s.enabled ? ic('pause') + ' Mettre en pause' : ic('play') + ' Reprendre la supervision',
      () => act('toggle', s.id));
  add(ic('star') + (s.favorite ? ' Retirer des favoris' : ' \u00c9pingler en favori'),
      () => setFavorite(s.id, !s.favorite));
  if (s.can_start) {
    add(ic('zap') + (s.auto_restart ? ' D\u00e9sactiver l\u2019autoboot' : ' Activer l\u2019autoboot'),
        () => setAuto(s.id, !s.auto_restart));
  }
  // Section Auth MCP Control (RFC 9421 / Ed25519 + variables d'env)
  add('—', null, false);
  if (s.transport === 'http') {
    if (s.paired) {
      add(ic('shield') + ' Auth MCP Control active \u2713 (' + esc(s.auth_keyid || 'mcc-server') + ')', null, true);
      add(ic('copy') + ' Copier la cl\u00e9 publique appair\u00e9e', () => copyMcpIdentity(s));
      add(ic('refresh') + ' R\u00e9-appairer l\u2019auth MCP Control', () => serverRepair(s.id));
      add(ic('trash') + ' Retirer l\u2019auth de ce serveur', () => serverUnpair(s.id));
    } else {
      add(ic('shield') + ' Appairer l\u2019auth MCP Control (Ed25519)', () => serverPair(s.id));
      add(ic('copy') + ' Copier l\u2019identit\u00e9 Ed25519 (cl\u00e9 publique)', () => copyMcpIdentity(s));
    }
  } else {
    add(ic('copy') + ' Copier l\u2019identit\u00e9 Ed25519 (cl\u00e9 publique)', () => copyMcpIdentity(s));
  }
  if (s.transport === 'stdio' || s.can_start) {
    const envOn = hasAuthEnv(s);
    add(
      ic('terminal') + (envOn ? ' Retirer l\u2019env d\u2019auth MCP Control' : ' Injecter l\u2019env d\u2019auth MCP Control'),
      () => toggleServerAuthEnv(s.id, !envOn)
    );
  }
  add('—', null, false);
  add(ic('eyeOff') + ' Masquer', () => hideServer(s.id));
  add(ic('trash') + ' Supprimer', () => (s.origin === 'seed' ? removeSeedForever(s.id) : removeSrv(s.id)));
  showMenu(ev, items);
}
// Affiche un menu d'entrées {lbl, fn, dis} à la position du curseur (via data-x/data-y : 0 style inline).
const CTX_X_STEPS = [16,48,80,112,144,176,208,240,272,304,336,368,400,448,512,576,640,704,768,832,896,960,1024,1120,1216,1312,1408,1504,1600,1632,1664,1696,1728,1760,1792,1824,1856,1888,1920,1952,1984,2016,2048,2080,2112,2144,2176,2208,2240,2272,2304,2336,2368,2400,2432,2464,2496];
const CTX_Y_STEPS = [16,48,80,112,144,176,208,240,272,304,336,368,400,448,512,576,640,704,768,832,864,896,928,960,992,1024,1056,1088,1120,1152,1184,1216,1248,1280,1312,1344,1376];
function snapStepFloor(val, steps) {
  let best = steps[0];
  for (let i = 0; i < steps.length; i++) {
    if (steps[i] <= val) best = steps[i];
    else break;
  }
  return best;
}
function showMenu(ev, items) {
  const m = document.getElementById('ctxmenu');
  if (!m) return;
  ctxOpener = document.activeElement;
  ctxItems = items;
  m.innerHTML = items.map((it, i) => it.lbl === '—'
    ? '<div class="ctxsep" role="separator"></div>'
    : '<div class="ctxitem' + (it.dis ? ' dis' : '') + (it.hdr ? ' ctx-hdr' : '') + '" role="menuitem" tabindex="-1" data-act="ctx-item" data-i="' + i + '"' + (it.dis ? ' aria-disabled="true"' : '') + '>' + it.lbl + '</div>').join('');
  m.hidden = false;
  const w = m.offsetWidth || 260, h = m.offsetHeight || 220;
  let cx = ev.clientX || 0, cy = ev.clientY || 0;
  if (!cx && !cy && ev.target && ev.target.getBoundingClientRect) {
    const r = ev.target.getBoundingClientRect();
    cx = r.left + 16;
    cy = r.bottom + 4;
  }
  const maxX = Math.max(16, window.innerWidth - w - 8);
  const maxY = Math.max(16, window.innerHeight - h - 8);
  const x = Math.max(16, Math.min(cx || 16, maxX));
  const y = Math.max(16, Math.min(cy || 16, maxY));
  m.dataset.x = String(snapStepFloor(x, CTX_X_STEPS));
  m.dataset.y = String(snapStepFloor(y, CTX_Y_STEPS));
  const first = m.querySelector('.ctxitem:not(.dis)');
  if (first) first.focus();
}
// Clic droit sur un ou plusieurs OUTILS du panneau de détail.
function formatToolInfoText(srvLabel, t) {
  const lines = [
    'outil       : ' + t.name + (t.disabled ? ' [OFF]' : ' [ON]'),
    'serveur     : ' + srvLabel,
  ];
  if (t.desc) lines.push('description : ' + String(t.desc).replace(/\s+/g, ' ').trim());
  if (t.input_schema) {
    lines.push('inputSchema :\n' + JSON.stringify(t.input_schema, null, 2));
  }
  return lines.join('\n');
}
function multiToolCtx(ev, id, names) {
  const s = (state && state.servers ? state.servers : []).find(x => x.id === id) || {};
  const d = (detailCache && detailCache.id === id) ? detailCache : {};
  const srvLabel = d.label || s.label || id;
  const all = d.tools_list || [];
  const tools = names.map(nm => all.find(x => x.name === nm) || {name: nm, desc: ''});
  const n = tools.length;
  const items = [];
  const add = (lbl, fn, dis) => items.push({lbl: lbl, fn: fn, dis: !!dis});
  items.push({lbl: ic('terminal') + ' ' + esc(n) + ' outils s\u00e9lectionn\u00e9s', fn: () => {}, dis: true, hdr: true});
  add('—', null, false);
  add(ic('copy') + ' Copier toutes les infos (' + n + ' outils)', () => {
    const txt = tools.map(t => formatToolInfoText(srvLabel, t)).join('\n\n' + '-'.repeat(40) + '\n\n');
    copyText(txt);
    toast('Infos de ' + n + ' outils copi\u00e9es', 'ok', 2200);
  });
  add(ic('copy') + ' Copier les sch\u00e9mas JSON (' + n + ' outils)', () => {
    const arr = tools.map(t => ({
      name: t.name,
      description: t.desc || '',
      disabled: !!t.disabled,
      inputSchema: t.input_schema || {type: 'object', properties: {}},
    }));
    copyText(JSON.stringify(arr, null, 2));
    toast('Sch\u00e9mas JSON de ' + n + ' outils copi\u00e9s', 'ok', 2200);
  });
  add(ic('copy') + ' Copier les noms (' + n + ')', () => {
    copyText(tools.map(t => t.name).join('\n'));
    toast(n + ' noms d\u2019outils copi\u00e9s', 'ok', 2000);
  });
  add(ic('copy') + ' Copier \u00ab serveur.outil \u00bb (' + n + ')', () => {
    copyText(tools.map(t => srvLabel + '.' + t.name).join('\n'));
    toast(n + ' r\u00e9f\u00e9rences copi\u00e9es', 'ok', 2000);
  });
  add('—', null, false);
  const offTools = tools.filter(t => t.disabled);
  if (offTools.length) {
    add(ic('check') + ' R\u00e9activer la s\u00e9lection (' + offTools.length + ')', async () => {
      for (const t of offTools) await postAction({action: 'toggle_tool', id, tool: t.name}, {quiet: true});
      const r = await fetch('/api/detail?id=' + encodeURIComponent(id));
      if (r.ok) { detailCache = await r.json(); renderDetail(); }
      await load();
      toast(offTools.length + ' outil(s) r\u00e9activ\u00e9(s)', 'ok', 2000);
    });
  }
  const onTools = tools.filter(t => !t.disabled);
  if (onTools.length) {
    add(ic('x') + ' D\u00e9sactiver la s\u00e9lection (' + onTools.length + ')', async () => {
      for (const t of onTools) await postAction({action: 'toggle_tool', id, tool: t.name}, {quiet: true});
      const r = await fetch('/api/detail?id=' + encodeURIComponent(id));
      if (r.ok) { detailCache = await r.json(); renderDetail(); }
      await load();
      toast(onTools.length + ' outil(s) d\u00e9sactiv\u00e9(s)', 'ok', 2000);
    });
  }
  add('—', null, false);
  add(ic('check') + ' Tout s\u00e9lectionner (affich\u00e9s)', () => {
    visibleToolNames().forEach(x => selectedTools.add(x));
    refreshToolsBox();
  });
  add(ic('x') + ' Effacer la s\u00e9lection', () => {
    selectedTools.clear();
    refreshToolsBox();
  });
  showMenu(ev, items);
}
function toolCtx(ev, id, name) {
  ev.preventDefault();
  ev.stopPropagation();
  if (selectedTools.size > 1 && selectedTools.has(name)) {
    const vis = visibleToolNames();
    const ordered = vis.filter(x => selectedTools.has(x));
    const rest = [...selectedTools].filter(x => !ordered.includes(x));
    multiToolCtx(ev, id, [...ordered, ...rest]);
    return;
  }
  // state.servers ne porte PAS tools_list : seul detailCache l'expose.
  const s = (state && state.servers ? state.servers : []).find(x => x.id === id) || {};
  const d = (detailCache && detailCache.id === id) ? detailCache : {};
  const t = (d.tools_list || []).find(x => x.name === name) || {name: name, desc: ''};
  const expKey = id + '::' + name;
  const isExp = expandedTools.has(expKey);
  const items = [];
  const add = (lbl, fn, dis) => items.push({lbl: lbl, fn: fn, dis: !!dis});
  add(ic('search') + (isExp ? ' Replier l\u2019inspecteur' : ' Inspecter le sch\u00e9ma & tester'), () => {
    if (expandedTools.has(expKey)) expandedTools.delete(expKey);
    else expandedTools.add(expKey);
    refreshToolsBox();
  });
  add('—', null, false);
  add(ic('copy') + " Copier le nom de l'outil", () => copyText(name));
  add(ic('copy') + ' Copier « serveur.outil »', () => copyText((d.label || s.label || id) + '.' + name));
  if (t.desc) add(ic('copy') + ' Copier la description', () => copyText(t.desc));
  add(ic('copy') + ' Copier le sch\u00e9ma JSON', () => copyToolSchema(name));
  add(ic('terminal') + " Copier l'appel JSON-RPC", () => copyToolRpc(name));
  if (ctxUrl(s)) add(ic('link') + " Copier l'URL + outil", () => copyText(ctxUrl(s) + '   # outil : ' + name));
  add('—', null, false);
  add(t.disabled ? ic('check') + ' Réactiver cet outil' : ic('x') + ' Désactiver cet outil',
      () => toggleTool(id, name));
  showMenu(ev, items);
}
// Clic droit : délégation (plus fiable que des attributs inline sur du HTML
// régénéré à chaque rendu). Ligne serveur → menu serveur, ligne outil → menu outil.
document.addEventListener('contextmenu', (ev) => {  const t = ev.target && ev.target.closest ? ev.target : null;
  if (!t) return;
  const row = t.closest('.srow');
  if (row && row.dataset.id) { ev.preventDefault(); ev.stopPropagation(); rowCtx(ev, row.dataset.id); return; }
  const tool = t.closest('.tool');
  if (tool && tool.dataset.tool && selectedId) {
    ev.preventDefault(); ev.stopPropagation(); toolCtx(ev, selectedId, tool.dataset.tool);
  }
});
// Diagnostic : une exception JS non capturée s'écrit dans le TITRE de la
// fenêtre (lisible depuis l'extérieur, ex. PowerShell MainWindowTitle) — le
// webview n'a pas de console accessible sur la build packagée.
window.addEventListener('error', (e) => {
  try { document.title = 'JSERR ' + (e.message || e.error || 'inconnu'); } catch (_) {}
});
window.addEventListener('unhandledrejection', (e) => {
  try { document.title = 'JSERR promesse ' + ((e.reason && e.reason.message) || e.reason || 'inconnu'); } catch (_) {}
});
/* Délégation centrale (lot S2) : un écouteur par TYPE d'événement sur le
   document — plus aucun attribut on* dans la page. Le menu contextuel se
   ferme au clic hors de lui ; l'action du contrôle cliqué passe par la table
   ACT (plus bas). Les plis <details> n'ont pas d'événement `toggle` qui
   bouillonne : on enregistre leur état après le clic sur <summary>.
   Ordre : tous les gestionnaires sont des déclarations hoistées, et ACT
   n'est lue qu'à l'événement — après l'exécution complète du script. */
document.addEventListener('click', (ev) => {
  const t = ev.target;
  if (!t || !t.closest) return;
  if (!t.closest('#ctxmenu')) ctxClose();
  runAction(ev);
});
document.addEventListener('toggle', (ev) => {
  const d = ev.target;
  if (d && d.matches && d.matches('details.fold[data-fold]')) saveFold(d);
}, true);
document.addEventListener('change', runAction);
document.addEventListener('input', runAction);
document.addEventListener('submit', (ev) => { ev.preventDefault(); runAction(ev); });
// Glisser-déposer : les événements HTML5 bouillonnent, on résout la cible au
// plus proche porteur de la classe attendue (ligne serveur ou en-tête de groupe).
const closestOf = (ev, sel) => (ev.target && ev.target.closest ? ev.target.closest(sel) : null);
document.addEventListener('dragstart', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) rowDragStart(ev, r, r.dataset.id);
});
document.addEventListener('dragover', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) { rowDragOver(ev, r); return; }
  const g = closestOf(ev, '.grphead[data-key]');
  if (g) groupDragOver(ev, g);
});
document.addEventListener('dragleave', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) rowDragLeave(ev, r);
  const g = closestOf(ev, '.grphead[data-key]');
  if (g) groupDragLeave(ev, g);
});
document.addEventListener('drop', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) { rowDrop(ev, r); return; }
  const g = closestOf(ev, '.grphead[data-key]');
  if (g) groupDrop(ev, g, g.dataset.key);
});
document.addEventListener('dragend', (ev) => {
  const r = closestOf(ev, '.srow[data-id]');
  if (r) rowDragEnd(ev, r);
});
/* Navigation clavier complète (WCAG 2.2 AA) :
   - Flèches ←/→/↑/↓ dans role="tablist" (Détail, Réglages, Accès)
   - Flèches ↑/↓, Entrée et Alt+↑/Alt+↓ (réordonner) sur les lignes .srow de l'explorateur
   - Ctrl+B : replier / déplier l'explorateur
   - Échap : ferme menu contextuel ou efface filtre et restaure le focus */
document.addEventListener('keydown', (e) => {
  if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'b') {
    e.preventDefault();
    toggleExplorer();
    return;
  }
  const el = document.activeElement;
  const ctxMenu = document.getElementById('ctxmenu');
  if (ctxMenu && !ctxMenu.hidden) {
    const items = [...ctxMenu.querySelectorAll('.ctxitem:not(.dis)')];
    const idx = items.indexOf(el);
    if (e.key === 'Escape') {
      e.preventDefault();
      ctxClose(true);
      return;
    }
    if (e.key === 'Tab') {
      ctxClose(false);
      return;
    }
    if (items.length && (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Home' || e.key === 'End')) {
      e.preventDefault();
      let next = 0;
      if (e.key === 'ArrowDown') next = idx >= 0 ? (idx + 1) % items.length : 0;
      else if (e.key === 'ArrowUp') next = idx >= 0 ? (idx - 1 + items.length) % items.length : items.length - 1;
      else if (e.key === 'End') next = items.length - 1;
      items[next].focus();
      return;
    }
    if ((e.key === 'Enter' || e.key === ' ') && idx >= 0) {
      e.preventDefault();
      items[idx].click();
      return;
    }
  }
  const discMenu = document.getElementById('discoverMenu');
  const tbDisc = document.getElementById('tbDiscover');
  if (discMenu && !discMenu.hidden) {
    const items = [...discMenu.querySelectorAll('.tb-menu-item:not([disabled])')];
    const idx = items.indexOf(el);
    if (e.key === 'Escape') {
      e.preventDefault();
      closeDiscoverMenu();
      if (tbDisc) tbDisc.focus();
      return;
    }
    if (e.key === 'Tab') {
      closeDiscoverMenu();
    } else if (items.length && (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Home' || e.key === 'End')) {
      e.preventDefault();
      let next = 0;
      if (e.key === 'ArrowDown') next = idx >= 0 ? (idx + 1) % items.length : 0;
      else if (e.key === 'ArrowUp') next = idx >= 0 ? (idx - 1 + items.length) % items.length : items.length - 1;
      else if (e.key === 'End') next = items.length - 1;
      items[next].focus();
      return;
    }
  } else if (el === tbDisc && e.key === 'ArrowDown') {
    e.preventDefault();
    toggleDiscoverMenu();
    return;
  }
  if ((e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) && el) {
    const rowCtxEl = el.closest ? el.closest('.srow[data-id]') : null;
    if (rowCtxEl) {
      e.preventDefault();
      rowCtx({preventDefault(){}, stopPropagation(){}, target: rowCtxEl}, rowCtxEl.dataset.id);
      return;
    }
    const toolCtxEl = el.closest ? el.closest('.tool[data-tool]') : null;
    if (toolCtxEl && selectedId) {
      e.preventDefault();
      toolCtx({preventDefault(){}, stopPropagation(){}, target: toolCtxEl}, selectedId, toolCtxEl.dataset.tool);
      return;
    }
  }
  if (el && el.getAttribute && el.getAttribute('role') === 'button' && el.dataset && el.dataset.act && (e.key === 'Enter' || e.key === ' ')) {
    e.preventDefault();
    el.click();
    return;
  }
  if (el && el.getAttribute && el.getAttribute('role') === 'tab') {
    const tablist = el.closest('[role="tablist"]');
    if (tablist && (e.key === 'ArrowRight' || e.key === 'ArrowLeft' || e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Home' || e.key === 'End')) {
      const tabs = [...tablist.querySelectorAll('[role="tab"]:not([disabled])')];
      const idx = tabs.indexOf(el);
      if (idx >= 0 && tabs.length > 1) {
        e.preventDefault();
        let next = idx;
        if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next = (idx + 1) % tabs.length;
        else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') next = (idx - 1 + tabs.length) % tabs.length;
        else if (e.key === 'Home') next = 0;
        else if (e.key === 'End') next = tabs.length - 1;
        const target = tabs[next];
        target.focus();
        target.click();
        return;
      }
    }
  }
  const row = el && el.closest ? el.closest('.srow[data-id]') : null;
  if (row) {
    if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown')) {
      e.preventDefault();
      const id = row.dataset.id;
      const localRows = [...document.querySelectorAll('#srvList .srow[data-id]')];
      const ids = localRows.map(r => r.dataset.id);
      const idx = ids.indexOf(id);
      const swapIdx = e.key === 'ArrowUp' ? idx - 1 : idx + 1;
      if (idx >= 0 && swapIdx >= 0 && swapIdx < ids.length) {
        const tmp = ids[idx]; ids[idx] = ids[swapIdx]; ids[swapIdx] = tmp;
        // Fusionner les serveurs masqués/filtrés pour ne jamais perdre leur ordre [M-16]
        const fullIds = [...ids];
        if (state && Array.isArray(state.servers)) {
          for (const s of state.servers) {
            if (!fullIds.includes(s.id)) fullIds.push(s.id);
          }
        }
        manualOrder = null;
        const sel = document.getElementById('sortSel');
        if (sel) sel.value = 'manuel';
        postReorder(fullIds).then(() => {
          const movedEl = document.querySelector(`#srvList .srow[data-id="${CSS.escape(id)}"]`);
          if (movedEl) movedEl.focus();
        });
      }
      return;
    }
    if (!e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown' || e.key === 'Home' || e.key === 'End')) {
      e.preventDefault();
      const allRows = [...document.querySelectorAll('#explorer .grpbody:not(.closed) .srow[data-id]')];
      if (!allRows.length) return;
      const idx = allRows.indexOf(row);
      let next = idx;
      if (e.key === 'ArrowDown') next = Math.min(allRows.length - 1, idx + 1);
      else if (e.key === 'ArrowUp') next = Math.max(0, idx - 1);
      else if (e.key === 'Home') next = 0;
      else if (e.key === 'End') next = allRows.length - 1;
      if (next >= 0 && next < allRows.length) allRows[next].focus();
      return;
    }
    if ((e.key === 'Enter' || e.key === ' ') && el === row) {
      e.preventDefault();
      select(row.dataset.id);
      return;
    }
  }
  if (e.key === 'Escape') {
    if (document.querySelector('dialog[open]')) return;
    if (selectedServers.size > 1 || selectedTools.size > 0) {
      if (selectedServers.size > 1) { selectedServers.clear(); renderServers(); }
      if (selectedTools.size > 0) { selectedTools.clear(); refreshToolsBox(); }
      return;
    }
    const f = document.getElementById('srvFilter');
    if (f && (f.value || document.activeElement === f)) {
      f.value = '';
      renderServers();
    }
    return;
  }
  const typing = el && (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA'
    || el.tagName === 'SELECT' || el.isContentEditable);
  if (!typing && (e.key === '/' || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k'))) {
    const openDlg = document.querySelector('dialog[open]');
    const storeDlg = document.getElementById('storeDlg');
    if (openDlg && (!storeDlg || openDlg !== storeDlg)) return;
    e.preventDefault();
    if (storeDlg && storeDlg.open) {
      const ss = document.getElementById('storeSearch');
      if (ss) { ss.focus(); ss.select(); }
      return;
    }
    const f = document.getElementById('srvFilter');
    if (f) { f.focus(); f.select(); }
  }
});
window.addEventListener('resize', ctxClose);

/* ===== Envoi d'action — UN SEUL chemin (lot C #1) =====
   postAction(body, opts) : POST /api/action, lit j.ok, affiche j.error en
   toast d'erreur et RENVOIE la réponse (les appelants réagissent à j.added,
   j.found, j.started…). Tout envoi d'action de la page passe par ici —
   contrat testé (plus aucun fetch d'action ailleurs). Les appelants qui
   affichent déjà le refus dans leur propre zone (messages de dialog, volet
   réglages, replis de mise à jour) passent {quiet: true} pour éviter un
   double signal. */
function netDown() {
  // Panne réseau constatée (erreur de transport, pas un refus) : bandeau
  // immédiat au lieu d'attendre le prochain cycle de sondage (lot C #3), et
  // poll passé en réessai rapide. Renvoie une réponse synthétique ok:false
  // pour que les appelants réagissent (repli downloadUpdate…).
  if (!document.getElementById('loginDlg')?.open) {
    pollFails = Math.max(pollFails, 1); // le prochain échec de poll compte comme 2ᵉ
    setDead(true, state === null ? DEAD_BOOT_MSG : DEAD_MSG);
    scheduleNext(POLL_RETRY_MS);
  }
  return {ok: false, error: 'moteur injoignable', netDown: true};
}
async function postAction(body, opts) {
  let r;
  try {
    r = await fetch('/api/action', {method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: JSON.stringify(body)});
  } catch (e) {
    return netDown(); // erreur réseau : bandeau immédiat (lot C #3)
  }
  let j = null;
  try { j = await r.json(); } catch (e) { /* corps non JSON : couvert plus bas */ }
  if (!j) {
    // pas de corps JSON (ex. 401 pendant le dialog du code d'accès) : le
    // dialog couvre déjà l'écran — pas de double signal.
    if (document.getElementById('loginDlg')?.open) return {ok: false, error: "code d'accès requis"};
    j = {ok: false, error: 'réponse illisible du moteur (HTTP ' + r.status + ')'};
  }
  if (!j.ok && !(opts && opts.quiet)) toast(j.error || 'échec', 'err'); // refus moteur : silence interdit (audit 04 #7)
  return j;
}
async function act(action, id) {
  const j = await postAction({action, id});
  if (!j.ok && j.needs_force && Array.isArray(j.occupants)) {
    const desc = j.occupants.map(o => (o.process || 'processus') + ' (PID ' + o.pid + ')').join(', ');
    const force = await askConfirm({
      title: 'Processus tiers sur le port',
      msg: 'Ce port est tenu par un processus qui n\u2019a pas \u00e9t\u00e9 lanc\u00e9 par MCP Control. Voulez-vous l\u2019arr\u00eater quand m\u00eame ?',
      detail: 'Occupant(s) : ' + desc,
      okLabel: 'Arr\u00eater quand m\u00eame',
      danger: true,
    });
    if (force) {
      const jf = await postAction({action: 'stop_server_force', id});
      if (jf && jf.ok) toast('Processus tiers arr\u00eat\u00e9', 'ok');
    }
  }
  await load();
}
function copyMcpIdentity(s) {
  const si = (state && state.server_identity) || null;
  const lines = [];
  if (s && s.paired && s.auth_public_key) {
    const kid = s.auth_keyid || 'mcc-server';
    lines.push('# Cl\u00e9 client appair\u00e9e pour ' + (s.label || s.id));
    lines.push('LITEMCP_TRUSTED_KEYS=' + kid + '=' + s.auth_public_key);
    lines.push('MCC_AUTH_KEYID=' + kid);
    lines.push('MCC_AUTH_PUBKEY=' + s.auth_public_key);
    if (s.auth_fingerprint) lines.push('# Empreinte : ' + s.auth_fingerprint);
  } else if (si && si.public_key) {
    const kid = si.keyid || 'mcc-server';
    lines.push('# Identit\u00e9 Ed25519 MCP Control');
    lines.push('LITEMCP_TRUSTED_KEYS=' + kid + '=' + si.public_key);
    lines.push('MCC_AUTH_KEYID=' + kid);
    lines.push('MCC_AUTH_PUBKEY=' + si.public_key);
    if (si.fingerprint) lines.push('# Empreinte : ' + si.fingerprint);
  }
  if (!lines.length) {
    toast('Identit\u00e9 Ed25519 indisponible', 'err');
    return;
  }
  copyText(lines.join('\n'));
}

async function serverPair(id) {
  const j = await postAction({action: 'server_pair', id});
  if (j && j.ok) {
    const kid = j.keyid || 'mcc-server';
    if (j.public_key) {
      copyText('LITEMCP_TRUSTED_KEYS=' + kid + '=' + j.public_key);
      toast('Appair\u00e9 (' + kid + ') \u2014 LITEMCP_TRUSTED_KEYS copi\u00e9e', 'ok');
    } else {
      toast('Serveur appair\u00e9 (' + kid + ')', 'ok');
    }
    await postAction({action: 'refresh', id}, {quiet: true});
  }
  await load();
  if (selectedId === id) {
    const r2 = await fetch('/api/detail?id=' + encodeURIComponent(id));
    if (r2.ok) { detailCache = await r2.json(); renderDetail(); }
  }
}

async function serverUnpair(id) {
  const ok = await askConfirm({
    title: 'Retirer l\u2019auth MCP Control',
    msg: 'Supprimer la cl\u00e9 Ed25519 appair\u00e9e pour ce serveur ? Les requ\u00eates ne seront plus sign\u00e9es RFC 9421.',
    okLabel: 'Retirer l\u2019auth',
    danger: true,
  });
  if (!ok) return;
  const j = await postAction({action: 'server_unpair', id});
  if (j && j.ok) {
    toast('Auth MCP Control retir\u00e9e', 'ok');
    await postAction({action: 'refresh', id}, {quiet: true});
  }
  await load();
  if (selectedId === id) {
    const r2 = await fetch('/api/detail?id=' + encodeURIComponent(id));
    if (r2.ok) { detailCache = await r2.json(); renderDetail(); }
  }
}

async function serverRepair(id) {
  const ok = await askConfirm({
    title: 'R\u00e9-appairer l\u2019auth MCP Control',
    msg: 'G\u00e9n\u00e9rer une nouvelle paire de cl\u00e9s Ed25519 pour ce serveur ? L\u2019ancienne cl\u00e9 sera remplac\u00e9e.',
    okLabel: 'R\u00e9-appairer',
  });
  if (!ok) return;
  const ju = await postAction({action: 'server_unpair', id});
  if (!ju || !ju.ok) return;
  await serverPair(id);
}

async function toggleServerAuthEnv(id, enabled) {
  const j = await postAction({action: 'server_auth_env', id, enabled: !!enabled});
  if (j && j.ok) {
    toast(
      enabled
        ? 'Variables d\u2019auth MCP Control inject\u00e9es (red\u00e9marrez le serveur pour appliquer)'
        : 'Variables d\u2019auth MCP Control retir\u00e9es',
      'ok'
    );
  }
  await load();
  if (selectedId === id) {
    const r2 = await fetch('/api/detail?id=' + encodeURIComponent(id));
    if (r2.ok) { detailCache = await r2.json(); renderDetail(); }
  }
}

// Autoboot par serveur (⚡) : relance auto au démarrage de MCP Control
// et après un crash. Persisté dans config.json par le moteur.
async function setAuto(id, enabled) {
  await postAction({action: 'set_auto_restart', id, enabled}); // refus toasté par le helper (audit 04 #7)
  if (selectedId === id) { detailCache = null; _detailEtag = null; _lastSelectedSrvSig = ''; }
  await load();
}

// Favori (⭐) : épingle la ligne en haut de sa section (persisté). L'épingle
// s'applique À L'AFFICHAGE — l'ordre posé au glisser-déposer reste en base.
async function setFavorite(id, enabled) {
  await postAction({action: 'set_favorite', id, enabled}); // refus toasté par le helper (audit 04 #7)
  if (selectedId === id) { detailCache = null; _detailEtag = null; _lastSelectedSrvSig = ''; }
  await load();
}

// Masquer de l'affichage (session seulement : la supervision moteur continue,
// mais la ligne ne s'affiche plus ni dans la liste ni dans les compteurs).
// Rien n'est persisté : la réouverture de l'app les fait revenir.
function hideServer(id) {
  hiddenServers.add(id);
  const n = hiddenServers.size;
  if (selectedId === id) { selectedId = null; detailCache = null; _sigDetail = ''; _sigOverview = ''; }
  render();
  toast(n + ' serveur' + (n > 1 ? 's' : '')
    + ' masqu\u00e9' + (n > 1 ? 's' : '')
    + ' \u2014 ' + (n > 1 ? 'ils reviennent' : 'il revient') + ' \u00e0 la r\u00e9ouverture de l\u2019app');
}
function showHiddenServers() {
  hiddenServers.clear();
  renderServers();
}

/* Scan en un clic : la plage vient des réglages (settings), aucune saisie demandée.
   Retour visuel (audit 04 #10) : bouton occupé (désactivé) pendant le scan,
   puis toast — nombre trouvé (champ `found` de la réponse, consommé s'il
   existe ; le lot 3 l'ajoutera), sinon message neutre. */
let scanBusy = false;
async function doScan() {
  if (scanBusy) return;
  scanBusy = true;
  const btns = [...document.querySelectorAll('button[data-act="scan"]')];
  btns.forEach(b => { b.disabled = true; });
  try {
    const j = await postAction({action: 'scan'}, {quiet: true}); // refus toasté plus bas avec son préfixe
    if (!j.ok) toast('scan : ' + (j.error || 'échec'), 'err');
    else if (Number.isFinite(j.found)) {
      toast(j.found + ' serveur' + (j.found > 1 ? 's' : '') + ' MCP trouvé' + (j.found > 1 ? 's' : ''),
        j.found > 0 ? 'ok' : undefined);
    } else toast('scan terminé — le journal détaille le résultat');
  } finally {
    scanBusy = false;
    btns.forEach(b => { b.disabled = false; });
  }
  await load();
}

async function doScanNet() {
  const btn = document.querySelector('[data-act="scan-net"]');
  if (btn) btn.disabled = true;
  try {
    const j = await postAction({action: 'scan_network'});
    if (j && j.ok) {
      toast(
        j.started
          ? 'Scan r\u00e9seau lanc\u00e9 en arri\u00e8re-plan \u2014 suivi dans le journal'
          : 'Scan r\u00e9seau termin\u00e9',
        'ok'
      );
    }
    await load();
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function refreshAll() {
  // UI-C02 : 1 seule action `refresh_all` côté moteur au lieu d'une rafale N+1
  // qui dépassait `MAX_CONNS_PER_IP = 16` dès 16 serveurs actifs.
  const btn = document.querySelector('[data-act="refresh-all"]');
  if (btn) { btn.disabled = true; btn.setAttribute('aria-busy', 'true'); }
  try {
    const j = await postAction({action: 'refresh_all'}, {quiet: true});
    if (!j || !j.ok) toast('rafra\u00eechissement global : ' + ((j && j.error) || '\u00e9chec'), 'err');
    await load();
  } finally {
    if (btn) { btn.disabled = false; btn.removeAttribute('aria-busy'); }
  }
}

async function doPeers() {
  await postAction({action: 'discover_peers'});
  await load();
}

async function importPeer(ip) {
  await postAction({action: 'import_peer', ip});
  await load();
}

/* ===== Machines distantes (SSH) ===== */
function openMachineDlg() {
  const d = document.getElementById('machineDlg');
  if (!d) return;
  if (!d.open) { rememberFocus(); d.showModal(); }
  renderMachines();
  const inp = document.getElementById('mIp');
  if (inp) inp.focus();
}

/* ===== Réglages du client (dialog ⚙ à volets) ===== */
/* Dirty state : tout input/change dans le dialog le marque ; fermer (Esc,
   ×, Annuler) avec des saisies pendantes demande confirmation au lieu de les
   jeter en silence. Le thème, lui, s'applique et se sauve au clic. */
let settingsDirty = false;
/* Instantané des valeurs notifications à l'ouverture du dialog : un
   Enregistrer ne POSTe set_notify que si la section a réellement bougé. */
let notifyOriginal = '';
function setDirty(v) {
  settingsDirty = v;
  const b = document.getElementById('settingsDirty');
  if (b) b.hidden = !v;
}
async function closeSettings() {
  if (settingsDirty) {
    const ok = await askConfirm({
      title: 'Modifications non enregistr\u00e9es',
      msg: 'Modifications non enregistr\u00e9es \u2014 fermer quand m\u00eame ?',
      okLabel: 'Abandonner',
      danger: true,
    });
    if (!ok) return;
  }
  setDirty(false);
  document.getElementById('settingsDlg').close();
  restoreFocus();
}
function setPane(name) {
  document.querySelectorAll('#settingsDlg .set-nav-btn').forEach(b => {
    const sel = b.dataset.pane === name;
    b.classList.toggle('sel', sel);
    b.setAttribute('aria-selected', sel ? 'true' : 'false');
    b.setAttribute('tabindex', sel ? '0' : '-1');
  });
  document.querySelectorAll('#settingsDlg .set-pane').forEach(p => p.classList.toggle('sel', p.dataset.pane === name));
  try { localStorage.setItem('mcc.setpane', name); } catch (e) {}
}
/* Labels humains des clés renvoyées par set_settings : le message parle
   utilisateur (« plage de scan »), pas clés internes (« scan »). */
const SETTINGS_LABELS = {
  refresh: 'intervalle de sondage',
  scan: 'plage de scan',
  ui_bind: '\u00e9coute',
  network: 'r\u00e9seau',
  network_ports: 'ports UI des pairs',
  discovery_ports_only: 'liste blanche de d\u00e9couverte',
  discovery_ports_excluded: 'ports exclus de la d\u00e9couverte',
};
const FIELD_ERR_META = {
  refresh:   {pane: 'general',       inputs: ['sRefresh']},
  scan:      {pane: 'decouverte',    inputs: ['sScanFrom', 'sScanTo']},
  ports:     {pane: 'decouverte',    inputs: ['sNetPorts']},
  disc_only: {pane: 'decouverte',    inputs: ['sDiscOnly']},
  disc_excl: {pane: 'decouverte',    inputs: ['sDiscExcl']},
  webhook:   {pane: 'notifications', inputs: ['sWebhookUrl']},
  states:    {pane: 'notifications', inputs: ['sStDown', 'sStPortOnly']},
  min:       {pane: 'notifications', inputs: ['sNotifMin']},
};
function setFieldErr(id, msg) {
  const el = document.getElementById('err_' + id);
  if (el) {
    if (msg) { el.textContent = msg; el.hidden = false; }
    else { el.textContent = ''; el.hidden = true; }
  }
  const meta = FIELD_ERR_META[id];
  if (meta) {
    meta.inputs.forEach(inpId => {
      const inp = document.getElementById(inpId);
      if (!inp) return;
      if (msg) inp.setAttribute('aria-invalid', 'true');
      else inp.removeAttribute('aria-invalid');
    });
  }
}
function clearFieldErrs() {
  ['refresh', 'scan', 'ports', 'disc_only', 'disc_excl', 'webhook', 'states', 'min'].forEach(id => setFieldErr(id, ''));
}
function showMsg(rowId, msgId, text) {
  const row = document.getElementById(rowId);
  const el = document.getElementById(msgId);
  if (row) row.hidden = false;
  if (el) el.textContent = text;
}

function openSettingsDlg() {
  const s = state.settings || {};
  const n = s.notify || {};
  document.getElementById('sRefresh').value = s.refresh_secs || 15;
  document.getElementById('sScanFrom').value = s.scan_from || 3000;
  document.getElementById('sScanTo').value = s.scan_to || 3200;
  document.getElementById('sNetEnabled').checked = !!s.network_enabled;
  document.getElementById('sNetPorts').value = (s.network_ports || []).join(', ');
  const discOnlyEl = document.getElementById('sDiscOnly');
  if (discOnlyEl) discOnlyEl.value = (s.discovery_ports_only || []).join(', ');
  const discExclEl = document.getElementById('sDiscExcl');
  if (discExclEl) discExclEl.value = (s.discovery_ports_excluded || []).join(', ');
  const bindSel = document.getElementById('sUiBind');
  const bindVal = s.ui_bind || '127.0.0.1';
  if (bindSel && ![...bindSel.options].some(o => o.value === bindVal)) {
    const opt = document.createElement('option');
    opt.value = bindVal;
    opt.textContent = bindVal + ' (personnalis\u00e9)';
    bindSel.appendChild(opt);
  }
  if (bindSel) bindSel.value = bindVal;
  document.getElementById('sUiPort').value = s.ui_port || 5090;
  // Notifications (moteur : action set_notify, section notify de config.json).
  // Lot 5 : les en-têtes ne sortent plus de l'API (headers_set) — le champ vide
  // signifie « inchangés » : le moteur ne reçoit la clé `headers` que si
  // l'utilisateur en saisit de nouvelles ; https:// est refusé par le moteur
  // (aucune pile TLS).
  document.getElementById('sNotifDesktop').checked = !!n.desktop;
  const urlField = document.getElementById('sWebhookUrl');
  urlField.value = n.webhook_url || '';
  urlField.placeholder = 'http://hooks.exemple.com/services/\u2026 (https non pris en charge)';
  document.getElementById('sWebhookMethod').value = n.method === 'PUT' ? 'PUT' : 'POST';
  const hdrField = document.getElementById('sWebhookHeaders');
  hdrField.value = '';
  hdrField.placeholder = n.headers_set
    ? 'En-t\u00eates d\u00e9j\u00e0 configur\u00e9s (masqu\u00e9s) \u2014 laisser vide = inchang\u00e9s'
    : 'Authorization: Bearer \u2026 (une paire \u00ab Cl\u00e9: valeur \u00bb par ligne)';
  const states = n.states || ['DOWN', 'PORT-ONLY'];
  const hasState = v => states.some(x => String(x).toUpperCase() === v);
  document.getElementById('sStDown').checked = hasState('DOWN');
  document.getElementById('sStPortOnly').checked = hasState('PORT-ONLY');
  document.getElementById('sNotifMin').value = (n.min_interval_secs ?? 300);
  notifyOriginal = JSON.stringify(notifyFormValues());
  // sVersion/sLatest sont des <span> (affichage), pas des champs éditables
  document.getElementById('sVersion').textContent = 'v' + MCC_VERSION;
  document.getElementById('sLatest').textContent = state.latest_version ? 'v' + state.latest_version : '\u2014';
  const upd = document.getElementById('updMsg');
  upd.textContent = state.update_available ? 'v' + state.latest_version + ' disponible' : 'Aucune mise \u00e0 jour connue (auto-v\u00e9rification au boot puis chaque heure).';
  upd.classList.remove('tone-warn', 'tone-ok');
  if (state.update_available) upd.classList.add('tone-warn');
  _updateAvailableKnown = !!state.update_available;
  const updBtn = document.getElementById('btnUpdInstall');
  const updActive = !!(_updatingNow || (state.update_status && state.update_status.active));
  if (updBtn && !updActive) {
    updBtn.disabled = !_updateAvailableKnown;
    updBtn.title = _updateAvailableKnown
      ? 'T\u00e9l\u00e9charge, v\u00e9rifie (SHA-256), installe et relance automatiquement la nouvelle version'
      : 'Aucune mise \u00e0 jour disponible pour le moment (v\u00e9rifie d\u2019abord ci-contre)';
  }
  if (state.update_status && state.update_status.phase && state.update_status.phase !== 'idle') {
    renderUpdateProgress(state.update_status);
  } else {
    setHidden(document.getElementById('updProgRow'), true);
  }
  const row = document.getElementById('resetMsgRow');
  if (row) row.hidden = true;
  document.getElementById('resetMsg').textContent = '';
  ['notifyMsgRow', 'runtimeMsgRow'].forEach(id => { const el = document.getElementById(id); if (el) el.hidden = true; });
  ['notifyMsg', 'runtimeMsg'].forEach(id => { const el = document.getElementById(id); if (el) el.textContent = ''; });
  document.getElementById('settingsMsg').textContent = (s.tls && s.tls.notice) ? s.tls.notice : '';
  clearFieldErrs();
  setDirty(false);
  // dernier volet consulté (préférence locale)
  let pane = 'general';
  try {
    const p = localStorage.getItem('mcc.setpane');
    const escP = p && (typeof CSS !== 'undefined' && CSS.escape ? CSS.escape(p) : p.replace(/[^a-z0-9_-]/gi, ''));
    if (escP && document.querySelector('#settingsDlg .set-pane[data-pane="' + escP + '"]')) pane = p;
  } catch (e) {}
  setPane(pane);
  const foot = document.getElementById('settingsFootNote');
  if (foot) foot.textContent = 'Les r\u00e9glages sont enregistr\u00e9s dans config.json.';
  applyTheme(document.documentElement.dataset.theme === 'light' ? 'light' : 'dark');
  applyTextStyle(document.documentElement.dataset.textColor || 'default', document.documentElement.dataset.textBright || '100');
  const d = document.getElementById('settingsDlg');
  if (d && !d.open) { rememberFocus(); d.showModal(); }
  const nav = document.querySelector('#settingsDlg .set-nav-btn.sel');
  if (nav) nav.focus();
}
async function checkUpdateNow() {
  const msg = document.getElementById('updMsg');
  const btn = document.querySelector('#settingsDlg [data-act="check-update"]');
  const updBtn = document.getElementById('btnUpdInstall');
  if (btn) { btn.disabled = true; btn.setAttribute('aria-busy', 'true'); }
  msg.classList.remove('tone-warn', 'tone-ok');
  msg.textContent = 'V\u00e9rification de la release GitHub\u2026';
  try {
    const j = await postAction({action: 'check_update'}, {quiet: true});
    const shaRow = document.getElementById('sSha256Row');
    const shaAsset = document.getElementById('sSha256Asset');
    const shaVal = document.getElementById('sSha256Val');
    const shaCopy = document.getElementById('sSha256Copy');
    const shaNote = document.getElementById('sSha256Note');
    if (shaRow) {
      if (j.sha256 && typeof j.sha256 === 'object') {
        shaRow.hidden = false;
        if (shaAsset) shaAsset.textContent = j.sha256.asset || 'binaire';
        if (j.sha256.sha256) {
          if (shaVal) shaVal.textContent = j.sha256.sha256;
          if (shaCopy) shaCopy.hidden = false;
          if (shaNote) shaNote.textContent = 'Empreinte SHA-256 publi\u00e9e dans SHA256SUMS.txt de la release GitHub.';
        } else {
          if (shaVal) shaVal.textContent = j.sha256.note || 'non publi\u00e9e';
          if (shaCopy) shaCopy.hidden = true;
          if (shaNote) shaNote.textContent = j.sha256.note || 'Empreinte non disponible pour cet actif.';
        }
      } else {
        shaRow.hidden = true;
      }
    }
    if (j.ok && j.checked && j.update_available) {
      _updateAvailableKnown = true;
      document.getElementById('sLatest').textContent = 'v' + j.latest;
      msg.textContent = 'v' + j.latest + ' disponible (install\u00e9e : v' + j.current + ') \u2014 clique \u00ab Mettre \u00e0 jour & relancer \u00bb';
      msg.classList.add('tone-warn');
    } else if (j.ok && j.checked) {
      _updateAvailableKnown = false;
      document.getElementById('sLatest').textContent = j.latest ? 'v' + j.latest : '\u2014';
      msg.textContent = '\u2713 \u00e0 jour \u2014 v\u00e9rifi\u00e9 \u00e0 l\u2019instant : v' + j.current + ' est la derni\u00e8re release';
      msg.classList.add('tone-ok');
    } else {
      msg.textContent = j.latest
        ? '\u2717 v\u00e9rification impossible (r\u00e9seau ?) \u2014 derni\u00e8re info connue : v' + j.latest + ' disponible'
        : '\u2717 v\u00e9rification impossible (r\u00e9seau ?) \u2014 nouvel essai auto au boot puis chaque heure';
    }
    if (updBtn && !_updatingNow) {
      updBtn.disabled = !_updateAvailableKnown;
      updBtn.title = _updateAvailableKnown
        ? 'T\u00e9l\u00e9charge, v\u00e9rifie (SHA-256), installe et relance automatiquement la nouvelle version'
        : 'Aucune mise \u00e0 jour disponible pour le moment (v\u00e9rifie d\u2019abord ci-contre)';
    }
  } finally {
    if (btn) { btn.disabled = false; btn.removeAttribute('aria-busy'); }
  }
}
function copySha256() {
  const el = document.getElementById('sSha256Val');
  if (el && el.textContent && el.textContent !== '\u2013') copyText(el.textContent);
}
async function openReleasePage() {
  // window.open est neutralisé dans la webview Tauri (desktop) : le clic mort
  // « rien ne se passe ». On passe par le moteur, qui lance le navigateur par
  // défaut de la machine (action open_release_page, URL fixe sans argument).
  // Repli : si le moteur ne peut pas (headless sans navigateur), on tente
  // window.open — utile quand l'UI tourne dans un vrai navigateur.
  const j = await postAction({action: 'open_release_page'}, {quiet: true});
  if (j.ok) return;
  // moteur injoignable ou refus : repli window.open (navigateur classique)
  window.open('https://github.com/DeamonDev888/mcp-control/releases/latest', '_blank', 'noopener,noreferrer');
}
/* OS du navigateur — la machine où atterrit le téléchargement quand l'UI est
   servie par un moteur distant (VPS). Le moteur valide et retombe sur le sien. */
function browserOs() {
  const ua = navigator.userAgent || '';
  if (ua.includes('Windows')) return 'windows';
  if (ua.includes('Mac OS') || ua.includes('Macintosh')) return 'macos';
  if (ua.includes('Linux') && !ua.includes('Android')) return 'linux';
  return null;
}
let lastUpdateUrl = null;
let _updatingNow = false;
let _updateAvailableKnown = false;
let _updateRestarting = false;
let _updateRestartSawDown = false;
let _updateTargetVer = '';
let _updateDismissedAt = 0;
let _lastUpdateErrorAt = 0;
let _lastUpdateRestartToast = false;

function fmtBytes(n) {
  const b = Number(n);
  if (!Number.isFinite(b) || b < 0) return '0 o';
  if (b < 1024) return Math.round(b) + ' o';
  if (b < 1024 * 1024) return (b / 1024).toFixed(1) + ' Ko';
  return (b / (1024 * 1024)).toFixed(1) + ' Mo';
}

function updatePhaseLabel(u) {
  if (!u) return '';
  if (u.message) return String(u.message);
  const ver = u.version ? (' v' + u.version) : '';
  switch (u.phase) {
    case 'checking': return 'V\u00e9rification de la derni\u00e8re release GitHub\u2026';
    case 'verifying_manifest': return 'V\u00e9rification de SHA256SUMS.txt' + ver + '\u2026';
    case 'downloading': return 'T\u00e9l\u00e9chargement' + ver + '\u2026';
    case 'verifying_sha256': return 'V\u00e9rification de l\u2019empreinte SHA-256\u2026';
    case 'installing': return 'Installation' + ver + '\u2026';
    case 'restarting': return 'Red\u00e9marrage vers la nouvelle version\u2026';
    case 'done': return 'Mise \u00e0 jour install\u00e9e.';
    case 'error': return '\u00c9chec : ' + (u.error || 'erreur inconnue');
    default: return '';
  }
}

function updateProgressMeta(u) {
  if (!u) return '';
  if (u.phase === 'downloading') {
    const dl = fmtBytes(u.bytes_downloaded || 0);
    const tot = (typeof u.bytes_total === 'number' && u.bytes_total > 0)
      ? (' / ' + fmtBytes(u.bytes_total)) : '';
    const pct = (typeof u.percent === 'number') ? (' \u00b7 ' + u.percent + ' %') : '';
    return dl + tot + pct;
  }
  if (u.phase === 'verifying_sha256' || u.phase === 'installing' || u.phase === 'restarting' || u.phase === 'done') {
    return u.version ? ('v' + u.version) : '';
  }
  return '';
}

function setProgressBar(bar, u) {
  if (!bar || !u) return;
  const st = String(u.phase || 'idle');
  bar.classList.toggle('upd-ok', st === 'restarting' || st === 'done');
  bar.classList.toggle('upd-err', st === 'error');
  if (st === 'downloading' && typeof u.percent === 'number') {
    bar.max = 100;
    bar.value = Math.max(0, Math.min(100, u.percent));
  } else if (st === 'restarting' || st === 'done') {
    bar.max = 100;
    bar.value = 100;
  } else if (st === 'error') {
    bar.max = 100;
    bar.value = 0;
  } else {
    bar.removeAttribute('value');
  }
}

function dismissUpdateBanner() {
  const u = state && state.update_status;
  if (u && u.active) return;
  _updateDismissedAt = (u && u.updated_at) ? u.updated_at : Date.now();
  setHidden(document.getElementById('updateBanner'), true);
}

async function fetchManualUpdateUrl() {
  const manual = {action: 'download_update'};
  const os = browserOs();
  if (os) manual.os = os;
  const dl = await postAction(manual, {quiet: true});
  if (dl && dl.url && /^https:\/\//i.test(dl.url)) lastUpdateUrl = dl.url;
}

function renderUpdateProgress(u) {
  const banner = document.getElementById('updateBanner');
  const bTxt = document.getElementById('updBannerTxt');
  const bMeta = document.getElementById('updBannerMeta');
  const bClose = document.getElementById('updBannerClose');
  const bProg = document.getElementById('updBannerProg');
  const row = document.getElementById('updProgRow');
  const sPhase = document.getElementById('updProgPhase');
  const sMeta = document.getElementById('updProgMeta');
  const sBar = document.getElementById('updProgBar');
  const msg = document.getElementById('updMsg');
  const btn = document.getElementById('btnUpdInstall');

  if (_updateRestarting && (!u || u.phase === 'idle')) {
    u = {
      active: true,
      phase: 'restarting',
      version: _updateTargetVer || null,
      bytes_downloaded: 0,
      bytes_total: null,
      percent: 100,
      message: 'Red\u00e9marrage vers la nouvelle version\u2026',
      error: null,
    };
  }

  if (!u || !u.phase || u.phase === 'idle') {
    _updatingNow = false;
    setHidden(banner, true);
    setHidden(row, true);
    if (btn) btn.disabled = !_updateAvailableKnown;
    return;
  }

  const phase = String(u.phase);
  const active = !!u.active;
  const isOk = phase === 'restarting' || phase === 'done';
  const isErr = phase === 'error';
  _updatingNow = active;
  if (u.version) _updateTargetVer = String(u.version);
  if (active) _updateDismissedAt = 0;

  if (phase === 'restarting') {
    _updateRestarting = true;
    if (!_lastUpdateRestartToast) {
      _lastUpdateRestartToast = true;
      toast('Mise \u00e0 jour v' + (u.version || _updateTargetVer || '') + ' v\u00e9rifi\u00e9e \u2014 red\u00e9marrage en cours\u2026', 'ok');
    }
  } else if (isErr) {
    _updatingNow = false;
    _updateRestarting = false;
    _updateRestartSawDown = false;
    _lastUpdateRestartToast = false;
    const errKey = u.updated_at || u.error || 'err';
    if (errKey !== _lastUpdateErrorAt) {
      _lastUpdateErrorAt = errKey;
      toast(u.error || '\u00e9chec de la mise \u00e0 jour', 'err');
      if (u.error && !String(u.error).includes('d\u00e9j\u00e0 \u00e0 jour') && !String(u.error).includes('SHA-256')) {
        fetchManualUpdateUrl();
      }
    }
  }

  const label = updatePhaseLabel(u);
  const meta = updateProgressMeta(u);
  const dismissed = !active && _updateDismissedAt && u.updated_at && _updateDismissedAt === u.updated_at;

  if (banner) {
    setHidden(banner, !!dismissed);
    banner.classList.toggle('upd-active', active);
    banner.classList.toggle('upd-ok', isOk);
    banner.classList.toggle('upd-err', isErr);
  }
  setText(bTxt, label);
  setText(bMeta, meta);
  setHidden(bClose, active);
  setProgressBar(bProg, u);

  setHidden(row, false);
  setText(sPhase, label);
  setText(sMeta, meta);
  setProgressBar(sBar, u);

  if (msg) {
    msg.classList.remove('tone-warn', 'tone-ok');
    if (isOk) msg.classList.add('tone-ok');
    else if (isErr) msg.classList.add('tone-warn');
    setText(msg, isErr ? ('\u2717 ' + (u.error || '\u00e9chec de la mise \u00e0 jour automatique')) : label);
  }
  if (btn) btn.disabled = !!(active || !_updateAvailableKnown);
}

/* Mettre à jour & relancer : téléchargement HTTPS + vérification SHA-256 +
   installation silencieuse (NSIS /S) ou remplacement binaire + redémarrage
   automatique de la nouvelle version avec suivi temps réel via /api/state. */
async function downloadUpdate() {
  if (_updatingNow || (state && state.update_status && state.update_status.active)) {
    openSettingsDlg();
    setPane('updates');
    return;
  }
  const ver = (state && state.latest_version) ? ('v' + state.latest_version) : 'la derni\u00e8re version';
  const ok = await askConfirm({
    title: 'Mettre \u00e0 jour et relancer',
    msg: 'T\u00e9l\u00e9charger ' + ver + ', v\u00e9rifier son empreinte SHA-256, installer automatiquement et relancer MCP Control ?',
    okLabel: 'Installer et relancer',
    cancelLabel: 'Annuler',
  });
  if (!ok) return;
  _updatingNow = true;
  _updateDismissedAt = 0;
  _updateRestartSawDown = false;
  _updateSessionClosed = false;
  _lastUpdateRestartToast = false;
  _updateTargetVer = (state && state.latest_version) ? String(state.latest_version) : '';
  renderUpdateProgress({
    active: true,
    phase: 'checking',
    version: _updateTargetVer || null,
    asset: null,
    bytes_downloaded: 0,
    bytes_total: null,
    percent: null,
    message: 'D\u00e9marrage de la mise \u00e0 jour ' + ver + '\u2026',
    error: null,
  });
  toast('Mise \u00e0 jour ' + ver + ' lanc\u00e9e\u2026', 'ok');
  const j = await postAction({action: 'install_update'}, {quiet: true});
  if (j.ok && (j.started || j.already_running)) {
    if (j.update_status) renderUpdateProgress(j.update_status);
    scheduleNext(POLL_UPDATE_MS);
    return;
  }
  _updatingNow = false;
  if (j.netDown) {
    setHidden(document.getElementById('updateBanner'), true);
    setHidden(document.getElementById('updProgRow'), true);
    if (lastUpdateUrl && /^https:\/\//i.test(lastUpdateUrl)) window.open(lastUpdateUrl, '_blank', 'noopener,noreferrer');
    else toast('moteur injoignable \u2014 r\u00e9essaie plus tard', 'err');
    return;
  }
  if (j.update_status) {
    renderUpdateProgress(j.update_status);
  } else {
    renderUpdateProgress({
      active: false,
      phase: 'error',
      version: _updateTargetVer || null,
      bytes_downloaded: 0,
      bytes_total: null,
      percent: 0,
      message: j.error || '\u00e9chec de la mise \u00e0 jour automatique',
      error: j.error || '\u00e9chec de la mise \u00e0 jour automatique',
      updated_at: Date.now(),
    });
  }
}
async function resetState(what) {
  const sel = document.getElementById('sResetWhat');
  const label = sel.options[sel.selectedIndex].text;
  const ok = await askConfirm({
    title: 'R\u00e9initialiser les donn\u00e9es',
    msg: 'R\u00e9initialiser \u00ab ' + label + ' \u00bb ? Action irr\u00e9versible.',
    okLabel: 'R\u00e9initialiser',
    danger: true,
  });
  if (!ok) return;
  const msg = document.getElementById('resetMsg');
  const row = document.getElementById('resetMsgRow');
  if (row) row.hidden = false;
  msg.textContent = 'R\u00e9initialisation\u2026';
  const j = await postAction({action: 'reset_state', what}, {quiet: true});
  if (!j.ok) msg.textContent = '✗ ' + (j.error || 'échec');
  else {
    const det = (j.reinjected ? ' · ' + j.reinjected + ' seed(s) réinjecté(s)' : '') + (j.removed ? ' · ' + j.removed + ' serveur(s) retiré(s)' : '');
    msg.textContent = '✓ Réinitialisé (' + (j.cleared || []).join(', ') + ')' + det;
    await load();
  }
}
/* Valeurs du formulaire notifications, telles quelles (sans le champ action) :
   sérialisées pour détecter si la section a réellement changé (évite un
   « configuration enregistrée » dans le journal à chaque Enregistrer). */
function notifyFormValues() {
  return {
    desktop: document.getElementById('sNotifDesktop').checked,
    webhook_url: document.getElementById('sWebhookUrl').value.trim(),
    method: document.getElementById('sWebhookMethod').value,
    // Lot 5 : champ vide = en-têtes inchangés — la clé n'est PAS envoyée et
    // le moteur conserve ceux déjà configurés (masqués côté API).
    headers: document.getElementById('sWebhookHeaders').value.trim() === ''
      ? undefined
      : document.getElementById('sWebhookHeaders').value,
    states: [
      ...(document.getElementById('sStDown').checked ? ['DOWN'] : []),
      ...(document.getElementById('sStPortOnly').checked ? ['PORT-ONLY'] : []),
    ],
    min_interval_secs: +document.getElementById('sNotifMin').value || 0,
  };
}
/* Lecture + validation de TOUS les champs du dialog : chaque erreur est
   affichée sous sa ligne (set-err), jamais une seule ligne globalisée.
   Renvoie null si invalide, sinon le corps d'action set_settings et, si la
   section Notifications a changé, le corps d'action set_notify. */
function collectSettingsForm() {
  let firstBad = '';
  const markErr = (id, msg) => {
    setFieldErr(id, msg);
    if (msg && !firstBad) firstBad = id;
  };
  const intOrNull = id => { const raw = document.getElementById(id).value.trim(); return raw === '' ? null : +raw; };
  // Intervalle de sondage : entier 5..3600
  const refresh = intOrNull('sRefresh');
  if (refresh === null || !Number.isInteger(refresh) || refresh < 5 || refresh > 3600) {
    markErr('refresh', 'entier entre 5 et 3600 secondes');
  } else markErr('refresh', '');
  // Plage de scan : entiers 1..65535, bornes échangées si inversées
  let sf = intOrNull('sScanFrom'), st = intOrNull('sScanTo');
  let swapped = false;
  if (sf === null || st === null || !Number.isInteger(sf) || !Number.isInteger(st)
      || sf < 1 || sf > 65535 || st < 1 || st > 65535) {
    markErr('scan', 'ports entiers entre 1 et 65535');
    sf = null; st = null;
  } else {
    if (sf > st) { const t = sf; sf = st; st = t; swapped = true; }
    markErr('scan', '');
  }
  // Ports UI des pairs : liste d'entiers 1..65535
  const rawPorts = document.getElementById('sNetPorts').value.trim();
  const tokens = rawPorts ? rawPorts.split(/[\s,]+/).filter(Boolean) : [];
  const bad = tokens.filter(t => !/^\d+$/.test(t) || +t < 1 || +t > 65535);
  if (bad.length) markErr('ports', 'ports invalides : ' + bad.slice(0, 3).join(', '));
  else markErr('ports', '');
  const netPorts = tokens.filter(t => /^\d+$/.test(t)).map(Number).filter(p => p >= 1 && p <= 65535);
  // Découverte locale : liste blanche (discovery_ports_only) et exclusions (discovery_ports_excluded)
  const parseDiscField = (inputId, errId) => {
    const el = document.getElementById(inputId);
    const raw = el ? el.value.trim() : '';
    const toks = raw ? raw.split(/[\s,]+/).filter(Boolean) : [];
    const badToks = toks.filter(t => !/^\d+$/.test(t) || +t < 1 || +t > 65535);
    if (badToks.length) { markErr(errId, 'ports invalides : ' + badToks.slice(0, 3).join(', ')); return []; }
    markErr(errId, '');
    return [...new Set(toks.map(Number).filter(p => p >= 1 && p <= 65535))];
  };
  const discOnly = parseDiscField('sDiscOnly', 'disc_only');
  const discExcl = parseDiscField('sDiscExcl', 'disc_excl');
  // Webhook : vide autorisé (canal off), sinon URL http:// uniquement —
  // https:// est refusé par le moteur à l'enregistrement (aucune pile TLS :
  // l'envoi partait en HTTP brut sur le port 443, audit 05 #4).
  const url = document.getElementById('sWebhookUrl').value.trim();
  if (url && !/^http:\/\//i.test(url)) markErr('webhook', 'URL http:// attendue (https non pris en charge : aucune pile TLS)');
  else markErr('webhook', '');
  // États notifiés : au moins un (sinon aucun incident ne déclencherait rien)
  const notify = notifyFormValues();
  notify.webhook_url = url;
  if (!notify.states.length) markErr('states', 'coche au moins un \u00e9tat (DOWN ou PORT-ONLY)');
  else markErr('states', '');
  // Délai anti-spam : borné 0..86400 (le moteur clampe déjà, on affiche juste)
  notify.min_interval_secs = Math.min(86400, Math.max(0, notify.min_interval_secs));
  if (firstBad) {
    const meta = FIELD_ERR_META[firstBad];
    if (meta) {
      setPane(meta.pane);
      const inp = document.getElementById(meta.inputs[0]);
      if (inp) inp.focus();
    }
    const msg = document.getElementById('settingsMsg');
    if (msg) msg.textContent = '\u2717 V\u00e9rifie les champs en erreur.';
    return null;
  }
  return {
    swapped,
    settings: {
      action: 'set_settings',
      refresh_secs: refresh,
      scan_from: sf,
      scan_to: st,
      ui_bind: document.getElementById('sUiBind').value,
      network_enabled: document.getElementById('sNetEnabled').checked,
      network_ports: netPorts,
      discovery_ports_only: discOnly,
      discovery_ports_excluded: discExcl,
    },
    notifyChanged: JSON.stringify(notify) !== notifyOriginal,
    notify,
  };
}
async function saveSettings() {
  const form = collectSettingsForm();
  if (!form) return false;
  const msg = document.getElementById('settingsMsg');
  const parts = [];
  const j = await postAction(form.settings, {quiet: true});
  if (!j.ok) { msg.textContent = '\u2717 ' + (j.error || '\u00e9chec'); return false; }
  parts.push((j.changed || []).length
    ? '\u2713 enregistr\u00e9 : ' + j.changed.map(k => SETTINGS_LABELS[k] || k).join(', ')
    : 'r\u00e9glages inchang\u00e9s');
  if ((j.changed || []).includes('ui_bind')) parts.push('l\u2019\u00e9coute s\u2019applique au prochain d\u00e9marrage');
  if (form.swapped) parts.push('plage de scan r\u00e9ordonn\u00e9e (du plus petit au plus grand port)');
  let allOk = true;
  if (form.notifyChanged) {
    const j2 = await postAction(Object.assign({action: 'set_notify'}, form.notify), {quiet: true});
    if (j2.ok) {
      parts.push('notifications enregistr\u00e9es');
      if (form.notify.headers !== undefined) {
        const hdrField = document.getElementById('sWebhookHeaders');
        if (hdrField) {
          hdrField.value = '';
          hdrField.placeholder = 'En-t\u00eates d\u00e9j\u00e0 configur\u00e9s (masqu\u00e9s) \u2014 laisser vide = inchang\u00e9s';
        }
      }
      notifyOriginal = JSON.stringify(notifyFormValues());
    } else {
      allOk = false;
      parts.push('\u2717 notifications : ' + (j2.error || '\u00e9chec'));
    }
  }
  msg.textContent = parts.join(' \u00b7 ');
  if (allOk) setDirty(false);
  await load();
  armPolling();
  return allOk;
}
/* Test des notifications : enregistre d'abord les saisies pendantes (le
   moteur teste la config PERSISTÉE), puis émet sur chaque canal activé. */
async function testNotify() {
  if (settingsDirty) { const saved = await saveSettings(); if (!saved) return; }
  showMsg('notifyMsgRow', 'notifyMsg', 'Test en cours\u2026');
  const j = await postAction({action: 'test_notify'}, {quiet: true});
  if (!j.ok) { showMsg('notifyMsgRow', 'notifyMsg', '\u2717 ' + (j.error || '\u00e9chec')); return; }
  const parts = [];
  if (j.desktop) parts.push(j.desktop.ok ? 'desktop \u2713 (toast affich\u00e9)' : 'desktop \u2717 \u2014 ' + (j.desktop.error || '?'));
  if (j.webhook) parts.push(j.webhook.ok ? 'webhook \u2713' : 'webhook \u2717 \u2014 ' + (j.webhook.error || '?'));
  showMsg('notifyMsgRow', 'notifyMsg', parts.join(' \u00b7 ') || 'aucun canal actif');
}
/* Historique runtime : crashs, relances, backoff — reset_runtime du moteur. */
async function resetRuntime() {
  const ok = await askConfirm({
    title: 'Effacer l\u2019historique runtime',
    msg: 'Effacer l\u2019historique runtime (crashs, relances, backoff) ?',
    okLabel: 'Effacer',
    danger: true,
  });
  if (!ok) return;
  const j = await postAction({action: 'reset_runtime'}, {quiet: true});
  showMsg('runtimeMsgRow', 'runtimeMsg',
    j.ok ? '\u2713 historique effac\u00e9 (' + (j.cleared || 0) + ' serveur(s))' : '\u2717 ' + (j.error || '\u00e9chec'));
  await load();
}
/* Câblage du dialog (une fois au boot) : dirty state sur toute saisie, et
   Esc = event cancel. */
(function () {
  const d = document.getElementById('settingsDlg');
  if (!d) return;
  const onEdit = e => {
    if (e.target && (e.target.id === 'sResetWhat' || e.target.id === 'sTextBright')) return;
    setDirty(true);
  };
  d.addEventListener('input', onEdit);
  d.addEventListener('change', onEdit);
  d.addEventListener('click', (e) => {
    if (e.target === d) closeSettings();
  });
  d.addEventListener('cancel', (e) => {
    e.preventDefault();
    closeSettings();
  });
})();
function renderMachines() {
  if (!state) return;
  const hosts = state.settings.ssh_hosts || [];
  const peers = state.peers || [];
  const box = document.getElementById('machineList');
  let html = hosts.length ? '' :
    '<div class="empty-sec">aucune machine enregistr&eacute;e — entre une IP et un utilisateur SSH ci-dessus</div>';
  html += hosts.map(h => {
    const peer = peers.find(p => p.ip === h.ip);
    const st = h.status || null;
    const hostDisp = (h.port && h.port !== 22) ? `${h.ip}:${h.port}` : h.ip;
    const osBadge = st && st.os
      ? `<span class="tag">${esc(st.os)}${st.arch ? '/' + esc(st.arch) : ''}</span>` : '';
    const cliBadge = !st ? ''
      : st.cli_running ? `<span class="tag ssh-cli-tag">CLI ${st.cli_version ? 'v' + esc(st.cli_version) + ' ' : ''}:${esc(String(st.cli_port || 5090))}</span>`
      : st.cli_installed ? `<span class="tag ssh-cli-tag">CLI ${st.cli_version ? 'v' + esc(st.cli_version) : 'pr&ecirc;t'} (arr&ecirc;t&eacute;)</span>`
      : '<span class="tag ssh-cli-tag">CLI absent</span>';
    const peerBadge = peer ? `<span class="tag sse">pair &middot; ${peer.via_ssh ? 'SSH' : 'LAN'}</span>` : '';
    const parts = [];
    if (peer) {
      parts.push(`moteur MCP Control d&eacute;tect&eacute; &mdash; ${esc((peer.stats||{}).up ?? '?')} UP / ${esc((peer.stats||{}).total ?? '?')} serveurs`);
    } else {
      parts.push('machine enregistr&eacute;e');
    }
    if (h.identity_file) parts.push(`cl&eacute; : ${esc(h.identity_file)}`);
    if (h.auto_install_cli === false) parts.push('auto-install CLI d&eacute;sactiv&eacute;e');
    if (st && st.last_error) parts.push(`note : ${esc(st.last_error)}`);
    const bootBtn = (!st || !st.cli_running)
      ? `<button type="button" data-act="ssh-bootstrap" data-id="${esc(h.ip)}" data-user="${esc(h.user)}">Installer / d&eacute;marrer CLI</button>`
      : '';
    return `<div class="machine-row">
      <div class="l1"><span class="dot ${peer ? 'UP' : 'IDLE'}"></span><span class="nm">${esc(hostDisp)}</span><span class="tag http">${esc(h.user)}</span>${osBadge}${cliBadge}${peerBadge}</div>
      <div class="l2">${parts.join(' &middot; ')}</div>
      <div class="macts">
        <button class="primary" data-act="ssh-discover" data-id="${esc(h.ip)}" data-user="${esc(h.user)}">${ic('link')} SSH &amp; d&eacute;couvrir</button>
        ${bootBtn}
        ${peer ? `<button data-act="import-peer" data-id="${esc(h.ip)}">Importer ses serveurs</button>` : ''}
        <button class="danger" data-act="ssh-forget" data-id="${esc(h.ip)}">Oublier</button>
      </div>
    </div>`;
  }).join('');
  const autres = peers.filter(p => !hosts.find(h => h.ip === p.ip));
  html += autres.map(p => `<div class="machine-row">
      <div class="l1"><span class="dot UP"></span><span class="nm">${esc(p.ip)}</span><span class="tag sse">pair &middot; ${p.via_ssh ? 'SSH' : 'LAN'}</span></div>
      <div class="l2">UI :${esc(p.ui_port)} &mdash; ${esc((p.stats||{}).up ?? '?')} UP / ${esc((p.stats||{}).total ?? '?')} serveurs</div>
      <div class="macts"><button data-act="import-peer" data-id="${esc(p.ip)}">Importer ses serveurs</button></div>
    </div>`).join('');
  box.innerHTML = html;
}
function sshForm() {
  const ip = (document.getElementById('mIp').value || '').trim();
  const user = (document.getElementById('mUser').value || '').trim();
  const portRaw = +(document.getElementById('mPort')?.value || 0);
  const port = Number.isInteger(portRaw) && portRaw >= 1 && portRaw <= 65535 && portRaw !== 22 ? portRaw : null;
  const identity_file = (document.getElementById('mKey')?.value || '').trim();
  const auto_install_cli = Boolean(document.getElementById('mAutoCli')?.checked ?? true);
  return {ip, user, port, identity_file, auto_install_cli};
}
async function sshDiscover(ip, user) {
  const fromForm = !ip;
  const f = sshForm();
  ip = ip || f.ip; user = user || f.user;
  if (!ip || !user) { toast('IP et utilisateur SSH requis', 'err', 3500); return; }
  if (_sshDiscoverIv) { clearInterval(_sshDiscoverIv); _sshDiscoverIv = null; }
  const msg = document.getElementById('machineMsg');
  const btns = [...document.querySelectorAll('#machineDlg [data-act="ssh-discover"]')];
  btns.forEach(b => { b.disabled = true; });
  msg.textContent = 'Connexion SSH + inspection OS/CLI + sonde des ports\u2026 (jusqu\u2019\u00e0 ~30 s, vois le journal)';
  try {
    const payload = {action: 'ssh_discover', ip, user};
    if (fromForm || (f.ip && f.ip === ip)) {
      if (f.port) payload.port = f.port;
      if (f.identity_file) payload.identity_file = f.identity_file;
      payload.auto_install_cli = f.auto_install_cli;
    }
    const j = await postAction(payload, {quiet: true});
    if (j.ok && j.started) {
      msg.textContent = 'D\u00e9couverte lanc\u00e9e sur ' + ip + ' \u2014 le journal et la liste se remplissent pendant la sonde.';
      let t = 0;
      _sshDiscoverIv = setInterval(async () => {
        const d = document.getElementById('machineDlg');
        if (!d || !d.open) { clearInterval(_sshDiscoverIv); _sshDiscoverIv = null; return; }
        await load(); renderMachines();
        if (++t >= 10) { clearInterval(_sshDiscoverIv); _sshDiscoverIv = null; }
      }, 4000);
    } else if (!j.ok) {
      msg.textContent = '\u2717 ' + (j.error || '\u00e9chec');
    }
  } finally {
    btns.forEach(b => { b.disabled = false; });
  }
}
async function sshBootstrap(ip, user) {
  const f = sshForm();
  ip = ip || f.ip; user = user || f.user;
  if (!ip || !user) { toast('IP et utilisateur SSH requis', 'err', 3500); return; }
  const msg = document.getElementById('machineMsg');
  const btns = [...document.querySelectorAll('#machineDlg [data-act="ssh-bootstrap"]')];
  btns.forEach(b => { b.disabled = true; });
  msg.textContent = 'V\u00e9rification / installation et d\u00e9marrage du CLI mcp-control sur ' + ip + '\u2026';
  try {
    const payload = {action: 'ssh_bootstrap', ip, user};
    if (f.ip && f.ip === ip) {
      if (f.port) payload.port = f.port;
      if (f.identity_file) payload.identity_file = f.identity_file;
    }
    const j = await postAction(payload, {quiet: true});
    if (j.ok && j.started) {
      msg.textContent = '\u2713 D\u00e9ploiement CLI lanc\u00e9 sur ' + ip + ' \u2014 suivi en direct dans le journal\u2026';
      let ticks = 0;
      const poll = setInterval(async () => {
        ticks++;
        await load();
        if (document.getElementById('machineDlg') && document.getElementById('machineDlg').open) {
          renderMachines();
        }
        if (ticks >= 10) clearInterval(poll);
      }, 3000);
    } else if (j.ok) {
      const osArch = [j.os, j.arch].filter(Boolean).join('/');
      const cliTxt = j.cli_running ? ('CLI actif :' + (j.cli_port || 5090)) : (j.cli_installed ? 'CLI install\u00e9' : 'CLI absent');
      msg.textContent = '\u2713 ' + ip + (osArch ? ' (' + osArch + ')' : '') + ' \u2014 ' + cliTxt + (j.error ? ' (' + j.error + ')' : '');
    } else {
      msg.textContent = '\u2717 ' + (j.error || 'bootstrap CLI \u00e9chou\u00e9');
    }
    await load(); renderMachines();
  } finally {
    btns.forEach(b => { b.disabled = false; });
  }
}
async function sshSave() {
  const f = sshForm();
  if (!f.ip || !f.user) { toast('IP et utilisateur SSH requis', 'err', 3500); return; }
  const payload = {action: 'ssh_save', ip: f.ip, user: f.user, auto_install_cli: f.auto_install_cli};
  if (f.port) payload.port = f.port;
  if (f.identity_file) payload.identity_file = f.identity_file;
  const j = await postAction(payload, {quiet: true});
  document.getElementById('machineMsg').textContent = j.ok ? '\u2713 machine enregistr\u00e9e' : '\u2717 ' + (j.error || 'erreur');
  await load(); renderMachines();
}
async function sshForget(ip) {
  const ok = await askConfirm({
    title: 'Oublier cette machine',
    msg: 'Oublier la machine ' + ip + ' ? Ses r\u00e9glages SSH enregistr\u00e9s seront retir\u00e9s.',
    okLabel: 'Oublier',
    danger: true,
  });
  if (!ok) return;
  await postAction({action: 'ssh_forget', ip});
  await load(); renderMachines();
}

async function toggleTool(id, name) {
  await postAction({action: 'toggle_tool', id, tool: name});
  const r = await fetch('/api/detail?id=' + encodeURIComponent(id));
  if (r.ok) { detailCache = await r.json(); renderDetail(); }
  await load();
}

function resetAddForm() {
  ['fLabel', 'fPort', 'fHttpHost', 'fHttpProgram', 'fHttpWorkdir', 'fHttpArgs', 'fProgram', 'fWorkdir', 'fArgs', 'fImport'].forEach(id => {
    const el = document.getElementById(id);
    if (el) el.value = '';
  });
  const sshEl = document.getElementById('fViaSsh');
  if (sshEl) sshEl.checked = false;
  const pathEl = document.getElementById('fPath');
  if (pathEl) pathEl.value = '/mcp';
  const trEl = document.getElementById('fTransport');
  if (trEl) trEl.value = 'http';
  transportChanged();
  const msg = document.getElementById('importMsg');
  if (msg) msg.textContent = '';
}
function toggleAdd() {
  const d = document.getElementById('dlg');
  if (!d) return;
  if (!d.open) { rememberFocus(); d.showModal(); }
  const msg = document.getElementById('importMsg');
  if (msg) msg.textContent = '';
  const lbl = document.getElementById('fLabel');
  if (lbl) lbl.focus();
}
function transportChanged() {
  const stdio = document.getElementById('fTransport').value === 'stdio';
  document.getElementById('httpFields').hidden = stdio;
  document.getElementById('stdioFields').hidden = !stdio;
}

/* Import d'un bloc mcpServers / URL / commande (audit 04 #4) : le moteur
   parse tout (import_mcp) ; l'UI affiche le nombre ajouté et, si la réponse
   les porte, les entrées écartées avec leur motif (champ `skipped`). */
async function importMcp() {
  const ta = document.getElementById('fImport');
  const msg = document.getElementById('importMsg');
  const btn = document.querySelector('#dlg [data-act="import-mcp"]');
  const text = (ta.value || '').trim();
  if (!text) { msg.textContent = '\u2717 colle un bloc mcpServers, une URL http:// ou une commande (npx, claude mcp add\u2026)'; return; }
  msg.textContent = 'Import en cours\u2026';
  if (btn) btn.disabled = true;
  try {
    const j = await postAction({action: 'import_mcp', text}, {quiet: true});
    if (!j.ok) { msg.textContent = '\u2717 ' + (j.error || '\u00e9chec'); return; }
    const parts = ['\u2713 ' + (j.added || 0) + ' serveur' + ((j.added || 0) > 1 ? 's' : '') + ' ajout\u00e9' + ((j.added || 0) > 1 ? 's' : '')];
    const skipped = Array.isArray(j.skipped) ? j.skipped : [];
    if (skipped.length) {
      parts.push('\u00e9cart\u00e9s : ' + skipped.map(it => typeof it === 'string' ? it
        : ((it.name || it.id || 'entr\u00e9e') + ((it.reason || it.error || it.motif) ? ' \u2014 ' + (it.reason || it.error || it.motif) : ''))).join(' ; '));
    }
    if (j.note) parts.push(j.note);
    await load();
    // succès complet : on referme (le toast rappelle le total) ; entrées
    // écartées ou rien d'ajouté : le dialog reste ouvert pour lire les motifs.
    if ((j.added || 0) > 0 && !skipped.length) {
      ta.value = '';
      msg.textContent = '';
      document.getElementById('dlg').close();
      restoreFocus();
      toast(parts.join(' \u2014 '), 'ok');
    } else {
      msg.textContent = parts.join(' \u2014 ');
    }
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function addServer(ev) {
  if (ev && ev.preventDefault) ev.preventDefault();
  const t = document.getElementById('fTransport').value;
  const label = (document.getElementById('fLabel').value || '').trim();
  if (!label) {
    toast('Nom affich\u00e9 requis', 'err', 3500);
    document.getElementById('fLabel').focus();
    return false;
  }
  const body = {action: 'add', transport: t, label};
  if (t === 'http') {
    const port = +document.getElementById('fPort').value;
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
      toast('Port HTTP invalide (1\u201365535)', 'err', 3500);
      document.getElementById('fPort').focus();
      return false;
    }
    body.port = port;
    body.path = (document.getElementById('fPath').value || '').trim() || '/mcp';
    const httpHost = (document.getElementById('fHttpHost')?.value || '').trim();
    if (httpHost) body.host = httpHost;
    if (document.getElementById('fViaSsh')?.checked) body.via_ssh = true;
    const prog = document.getElementById('fHttpProgram').value.trim();
    if (prog) {
      body.program = prog;
      const wd = document.getElementById('fHttpWorkdir').value.trim();
      if (wd) body.workdir = wd;
      body.args = document.getElementById('fHttpArgs').value.split(/\s+/).filter(Boolean);
    }
  } else {
    const prog = (document.getElementById('fProgram').value || '').trim();
    if (!prog) {
      toast('Programme requis en mode STDIO', 'err', 3500);
      document.getElementById('fProgram').focus();
      return false;
    }
    body.program = prog;
    body.workdir = document.getElementById('fWorkdir').value.trim() || null;
    body.args = document.getElementById('fArgs').value.split(/\s+/).filter(Boolean);
  }
  const subBtn = document.querySelector('#dlg button[type="submit"]');
  if (subBtn) subBtn.disabled = true;
  try {
    const j = await postAction(body, {quiet: true});
    if (!j.ok) toast(j.error || 'erreur', 'err', 4000);
    else {
      resetAddForm();
      document.getElementById('dlg').close();
      restoreFocus();
      if (j.id) {
        await act('refresh', j.id);
        await select(j.id);
      } else {
        await load();
      }
    }
  } finally {
    if (subBtn) subBtn.disabled = false;
  }
  return false;
}

/* ===== Boutique MCP (porté de Hermes mcp-import.ts / connectors-directory) ===== */
let storeItems = null;
let storeLogos = null;
let storeVersions = null;
let _storeVersionsFetched = false;
let storeCat = 'Tout';
function fmtStoreVer(v) {
  const s = String(v || '').trim();
  if (!s) return '';
  return /^v/i.test(s) ? s : 'v' + s;
}
async function checkStoreVersions(btn) {
  if (btn) {
    btn.disabled = true;
    btn.innerHTML = '<span class="spin">' + ic('refresh') + '</span> V\u00e9rification\u2026';
  }
  try {
    const r = await fetch('/api/store-versions?refresh=1');
    if (!r.ok) throw new Error('HTTP ' + r.status);
    const v = await r.json();
    if (v && typeof v === 'object') {
      storeVersions = Object.assign(storeVersions || {}, v);
      renderStoreItems();
      renderServers();
      const n = Object.keys(storeVersions).length;
      toast('Versions v\u00e9rifi\u00e9es (' + n + ' serveurs)', 'ok', 2800);
    }
  } catch (e) {
    toast('V\u00e9rification des versions impossible', 'err', 3200);
  } finally {
    if (btn) {
      btn.disabled = false;
      btn.textContent = 'V\u00e9rifier versions';
    }
  }
}
async function openStore() {
  const d = document.getElementById('storeDlg');
  if (!d) return;
  if (!d.open) { rememberFocus(); d.showModal(); }
  const inp = document.getElementById('storeSearch');
  if (inp) inp.focus();
  if (!storeItems) {
    const grid = document.getElementById('storeGrid');
    if (grid) grid.innerHTML = '<div class="empty-sec"><span class="spin">' + ic('refresh') + '</span> Chargement de la boutique\u2026</div>';
    try {
      const r = await fetch('/api/store');
      if (!r.ok) throw new Error('HTTP ' + r.status);
      storeItems = await r.json();
      document.getElementById('storeCount').textContent = '(' + storeItems.length + ' MCP)';
      // logos officiels (glyphes embarqués au binaire, servis en JSON)
      fetch('/api/store-logos').then(r2 => r2.ok ? r2.json() : null).then(j => {
        if (!j) return;
        storeLogos = j;
        renderStoreItems(); // re-rendu quand les logos arrivent
      }).catch(() => {});
    } catch (e) {
      if (grid) grid.innerHTML = '<div class="empty-sec">Impossible de charger la boutique MCP.<div class="empty-acts"><button type="button" data-act="store">R\u00e9essayer</button></div></div>';
      return;
    }
  }
  if (!storeVersions) {
    fetch('/api/store-versions').then(r3 => r3.ok ? r3.json() : null).then(v => {
      if (!v) return;
      storeVersions = v;
      renderStoreItems();
    }).catch(() => {});
  }
  renderStoreItems();
}
/* Logo d'une carte : glyphe officiel par id (STORE_LOGOS), sinon pour les items
   aitmpl une marque reconnue dans la commande d'installation, sinon un glyphe
   « paquet » neutre — TOUTE carte porte une icône (jamais de carte sans logo,
   avant comme après le chargement de /api/store-logos). NB : guillemet-double
   suivi de dièse interdit dans ce bloc — la séquence fermerait la raw string
   Rust qui embarque cette page. */
const PKG_GLYPH = "<path fill='none' stroke='#8a93a5' stroke-width='2' stroke-linejoin='round' d='m12 2 9 5v10l-9 5-9-5V7Zm0 10 9-5m-9 5-9-5m9 5v10'/>";
const PKG_LOGOS = [
  ['context7', 'context7'], ['mongodb', 'mongodb'], ['supabase', 'supabase'],
  ['playwright', 'playwright'], ['figma', 'figma'], ['firecrawl', 'firecrawl'],
  ['notionhq', 'notion'], ['stripe', 'stripe'], ['kubernetes', 'kubernetes'],
  ['duckduckgo', 'duckduckgo'], ['todoist', 'todoist'], ['sqlite', 'sqlite'],
  ['postgres', 'postgresql'], ['server-gdrive', 'googledrive'], ['slack', 'slack'],
  ['brave', 'brave'], ['n8n', 'n8n'], ['gitlab', 'gitlab'], ['sentry', 'sentry'],
  ['server-google-maps', 'googlemaps'], ['paddleocr', 'paddlepaddle'],
  ['chrome-devtools', 'googlechrome'],
];
function storeLogoSvg(inner) {
  return `<span class="slogo-well${logoContrastCls(inner)}"><svg class="slogo" viewBox="0 0 24 24" aria-hidden="true">${inner}</svg></span>`;
}
function storeLogo(i) {
  if (i && i.id === 'swarm-control') {
    return `<span class="slogo-well swarm-slogo-well"><img class="swarm-img" src="${esc(swarmLogoUrl('logo', 'hive'))}" alt="" aria-hidden="true"></span>`;
  }
  if (!storeLogos) return storeLogoSvg(PKG_GLYPH);
  // glyphe officiel par id, préfixes de catalogues additionnels retirés (BT-C06)
  const inner = storeLogos[i.id] || storeLogos[i.id.replace(/^(at|gl|gh)-/, '')];
  if (inner) return storeLogoSvg(inner);
  // items de catalogues additionnels : marque reconnue dans la commande
  if (/^(at|gl|gh)-/.test(i.id)) {
    const cmd = (i.program + ' ' + (i.args || []).join(' ')).toLowerCase();
    for (const [needle, lid] of PKG_LOGOS) {
      if (storeLogos[lid] && cmd.includes(needle)) return storeLogoSvg(storeLogos[lid]);
    }
  }
  // glyphe « paquet » neutre pour tout ce qui n'a pas de marque reconnue
  return storeLogoSvg(PKG_GLYPH);
}
/* Id serveur d'un item boutique : install_store clé sur « store-<id> ». */
const storeSrvId = i => 'store-' + i.id;
let storeHideInst = false;
function toggleHideInst() {
  storeHideInst = !storeHideInst;
  const b = document.getElementById('storeHideInst');
  if (b) {
    b.classList.toggle('sel', storeHideInst);
    b.setAttribute('aria-checked', storeHideInst ? 'true' : 'false');
  }
  renderStoreItems();
}
function storeEnvKeys(i) {
  if (i && Array.isArray(i.env_vars)) {
    return i.env_vars;
  }
  const desc = String((i && i.desc) || '');
  const matches = desc.match(/\b[A-Z][A-Z0-9_]{3,}\b/g) || [];
  const keys = [...new Set(matches.filter(k => k.includes('_')))];
  if (!keys.length && /tokens?\s+requis\s+en\s+env/i.test(desc)) {
    return ['API_TOKEN'];
  }
  return keys;
}
function toggleEnvEye(btn) {
  const row = btn && btn.closest ? btn.closest('.scard-env-row') : null;
  const inp = row ? row.querySelector('input.store-env-inp') : null;
  if (!inp) return;
  const show = inp.type === 'password';
  inp.type = show ? 'text' : 'password';
  btn.innerHTML = ic(show ? 'eyeOff' : 'eye');
  btn.setAttribute('aria-pressed', show ? 'true' : 'false');
}
function clearStoreSearch() {
  const inp = document.getElementById('storeSearch');
  if (inp) { inp.value = ''; inp.focus(); }
  renderStoreItems();
}
function resetStoreFilters() {
  storeCat = 'Tout';
  storeHideInst = false;
  const b = document.getElementById('storeHideInst');
  if (b) {
    b.classList.remove('sel');
    b.setAttribute('aria-checked', 'false');
  }
  const inp = document.getElementById('storeSearch');
  if (inp) inp.value = '';
  const sort = document.getElementById('storeSort');
  if (sort) sort.selectedIndex = 0;
  renderStoreItems();
  if (inp) inp.focus();
}
function renderStoreCats(baseList) {
  const box = document.getElementById('storeCats');
  if (!box || !storeItems) return;
  const counts = {};
  (baseList || storeItems).forEach(i => { counts[i.cat] = (counts[i.cat] || 0) + 1; });
  const cats = ['Tout', ...new Set(storeItems.map(i => i.cat))];
  box.innerHTML = cats.map(c => {
    const n = c === 'Tout' ? (baseList || storeItems).length : (counts[c] || 0);
    const sel = c === storeCat;
    return `<button type="button" class="scat${sel ? ' sel' : ''}" data-act="store-cat" data-cat="${esc(c)}" aria-pressed="${sel ? 'true' : 'false'}">${esc(c)} <span class="scat-n">(${esc(n)})</span></button>`;
  }).join('');
}
function renderStoreItems() {
  if (!storeItems) return;
  const searchEl = document.getElementById('storeSearch');
  const rawQ = (searchEl && searchEl.value) || '';
  const clr = document.getElementById('storeSearchClear');
  if (clr) clr.hidden = !rawQ;
  const q = rawQ.toLowerCase();
  const sort = document.getElementById('storeSort').value;
  const instMap = new Map((state && state.servers ? state.servers : []).map(s => [s.id, s]));
  const baseList = storeItems.filter(i => {
    if (storeHideInst && instMap.has(storeSrvId(i))) return false;
    if (!q) return true;
    return (
      i.name.toLowerCase().includes(q)
      || i.desc.toLowerCase().includes(q)
      || i.id.toLowerCase().includes(q)
      || (i.cat && i.cat.toLowerCase().includes(q))
      || (i.kind && i.kind.toLowerCase().includes(q))
      || (i.program && i.program.toLowerCase().includes(q))
      || (Array.isArray(i.args) && i.args.join(' ').toLowerCase().includes(q))
      || (Array.isArray(i.env_vars) && i.env_vars.join(' ').toLowerCase().includes(q))
    );
  });
  renderStoreCats(baseList);
  let list = storeCat === 'Tout' ? baseList : baseList.filter(i => i.cat === storeCat);
  if (sort === 'az') list = [...list].sort((a, b) => a.name.localeCompare(b.name, 'fr', {sensitivity: 'base'}));
  const res = document.getElementById('storeResults');
  res.textContent = list.length === storeItems.length
    ? storeItems.length + ' serveurs'
    : list.length + ' affich\u00e9' + (list.length > 1 ? 's' : '') + ' sur ' + storeItems.length;
  const grid = document.getElementById('storeGrid');
  const drafts = new Map();
  if (grid) {
    grid.querySelectorAll('.scard[data-store-id]').forEach(card => {
      const sid = card.dataset.storeId;
      card.querySelectorAll('input.store-env-inp').forEach(inp => {
        if (inp.value || inp.type === 'text') {
          drafts.set(sid + '::' + (inp.dataset.env || ''), {value: inp.value, show: inp.type === 'text'});
        }
      });
    });
  }
  grid.innerHTML = list.map(i => {
    const instSrv = instMap.get(storeSrvId(i));
    const isInst = Boolean(instSrv);
    const remoteVer = (storeVersions && storeVersions[i.id]) || i.version || '';
    const instVer = (instSrv && instSrv.version) || '';
    const hasUpd = Boolean(isInst && instVer && remoteVer && instVer.replace(/^v/i, '') !== remoteVer.replace(/^v/i, ''));
    const verHtml = hasUpd
      ? `<span class="scard-ver scard-ver-upd" title="Install\u00e9e : ${esc(fmtStoreVer(instVer))} \u2014 Derni\u00e8re version : ${esc(fmtStoreVer(remoteVer))}">${esc(fmtStoreVer(instVer))} \u2192 ${esc(fmtStoreVer(remoteVer))}</span>`
      : (remoteVer || instVer)
        ? `<span class="scard-ver" title="${isInst && instVer && remoteVer ? '\u00c0 jour (' + esc(fmtStoreVer(remoteVer)) + ')' : 'Version du serveur MCP'}">${esc(fmtStoreVer(remoteVer || instVer))}</span>`
        : '';
    const kindCls = i.kind === 'npm' ? 'npm' : i.kind === 'rust' ? 'rust' : 'py';
    const envKeys = !isInst ? storeEnvKeys(i) : [];
    const envHtml = envKeys.length > 2
      ? `<div class="scard-env-hint" title="${esc(envKeys.join(', '))}">${ic('lock')} ${esc(envKeys.length)} variables demand\u00e9es \u00e0 l\u2019installation</div>`
      : envKeys.length
        ? `<div class="scard-env">${envKeys.map(k =>
            `<div class="scard-env-row">
              <input type="password" class="store-env-inp" data-env="${esc(k)}" placeholder="${esc(k)}" aria-label="Variable ${esc(k)}" autocomplete="off">
              <button type="button" class="eye-btn" data-act="toggle-env-eye" aria-pressed="false" aria-label="Afficher ou masquer ${esc(k)}" title="Afficher / masquer">${ic('eye')}</button>
            </div>`).join('')}</div>`
        : '';
    return `<div class="scard" data-store-id="${esc(i.id)}">
      <div class="scard-top">
        ${storeLogo(i)}
        <div class="scard-title">
          <b>${esc(i.name)}</b>
          <span class="scard-meta"><span class="scard-cat">${esc(i.cat)}</span>${verHtml}</span>
        </div>
        <span class="tag ${kindCls}">${esc(i.kind)}</span>
      </div>
      <div class="scard-desc">${esc(i.desc)}</div>
      ${envHtml}
      <div class="scard-foot">
        ${isInst
          ? (hasUpd
              ? `<button type="button" class="btn-upd" data-act="update-store" data-id="${esc(i.id)}" title="Mettre \u00e0 jour ${esc(i.name)} (${esc(fmtStoreVer(instVer))} \u2192 ${esc(fmtStoreVer(remoteVer))})">${ic('download')} Mettre \u00e0 jour</button><button type="button" class="danger btn-uninst" data-act="uninstall-store" data-id="${esc(i.id)}" title="Supprimer ce serveur et tous ses fichiers locaux">${ic('trash')} Supprimer</button>`
              : `<span class="scard-inst-badge" title="Ce serveur est d\u00e9j\u00e0 dans ta liste">${ic('check')} Install\u00e9</span><button type="button" class="danger btn-uninst" data-act="uninstall-store" data-id="${esc(i.id)}" title="Supprimer ce serveur et tous ses fichiers locaux">${ic('trash')} Supprimer</button>`)
          : `<button type="button" class="primary" data-act="install-store" data-id="${esc(i.id)}">${ic('download')} Installer</button>`}
      </div>
    </div>`;
  }).join('') || '<div class="empty-sec">Aucun r\u00e9sultat.<div class="empty-acts"><button type="button" data-act="store-reset-filters">R\u00e9initialiser les filtres</button></div></div>';
  if (drafts.size) {
    grid.querySelectorAll('.scard[data-store-id]').forEach(card => {
      const sid = card.dataset.storeId;
      card.querySelectorAll('input.store-env-inp').forEach(inp => {
        const saved = drafts.get(sid + '::' + (inp.dataset.env || ''));
        if (!saved) return;
        inp.value = saved.value;
        if (saved.show) {
          inp.type = 'text';
          const eye = inp.parentElement && inp.parentElement.querySelector('.eye-btn');
          if (eye) { eye.innerHTML = ic('eyeOff'); eye.setAttribute('aria-pressed', 'true'); }
        }
      });
    });
  }
}
async function installStore(id, btn) {
  const item = storeItems && storeItems.find(x => x.id === id);
  const envKeys = item ? storeEnvKeys(item) : [];
  const card = btn && btn.closest ? btn.closest('.scard') : null;
  const envInps = card ? [...card.querySelectorAll('input.store-env-inp')] : [];
  const envMap = {};
  let hasEnv = false;
  envInps.forEach(inp => {
    const k = inp.dataset.env;
    const v = (inp.value || '').trim();
    if (k && v) { envMap[k] = v; hasEnv = true; }
  });
  if (envKeys.length > 2) {
    const res = await askConfirm({
      title: 'Configuration \u2014 ' + ((item && item.name) || id),
      msg: 'Renseignez les variables d\u2019environnement requises pour ce serveur (les champs laiss\u00e9s vides seront ignor\u00e9s) :',
      inputs: envKeys.map(k => ({id: k, label: k, type: 'password', placeholder: k})),
      okLabel: 'Installer',
    });
    if (!res || typeof res !== 'object') return;
    envKeys.forEach(k => {
      const v = (res[k] || '').trim();
      if (v) { envMap[k] = v; hasEnv = true; }
    });
  }
  btn.disabled = true;
  btn.innerHTML = '<span class="spin">' + ic('refresh') + '</span> Installation\u2026';
  const body = {action: 'install_store', id};
  if (hasEnv) body.env = envMap;
  if (id === 'postgres') {
    const res = await askConfirm({
      title: 'URL PostgreSQL',
      msg: 'Indiquez l\u2019URL de connexion PostgreSQL (\u00e0 passer en argument, acc\u00e8s lecture seule) :',
      inputs: [{id: 'url', label: 'URL PostgreSQL', placeholder: 'postgresql://user:pass@localhost:5432/mydb'}],
      okLabel: 'Installer',
    });
    const url = res && typeof res === 'object' ? res.url : '';
    if (!url || !url.trim()) {
      btn.disabled = false; btn.innerHTML = ic('download') + ' Installer';
      return;
    }
    // BT-C01 : le backend concatène déjà `it.args` (`-y`, `@modelcontextprotocol/server-postgres`).
    body.args = [url.trim()];
  }
  const j = await postAction(body, {quiet: true});
  if (j.ok) {
    envInps.forEach(inp => { inp.value = ''; inp.type = 'password'; });
    if (j.version) {
      if (!storeVersions) storeVersions = {};
      storeVersions[id] = j.version;
    }
    const nm = (storeItems && storeItems.find(x => x.id === id) || {}).name || id;
    toast(nm + (j.version ? ' ' + fmtStoreVer(j.version) : '') + ' install\u00e9 avec succ\u00e8s', 'ok');
    await load();
    renderStoreItems();
  } else {
    btn.disabled = false;
    btn.innerHTML = ic('refresh') + ' R\u00e9essayer';
    toast(j.error || 'erreur', 'err', 4000);
  }
}
async function openStoreToUpdate(storeId) {
  if (!storeId) return;
  storeCat = 'Tout';
  storeHideInst = false;
  const hideBtn = document.getElementById('storeHideInst');
  if (hideBtn) {
    hideBtn.classList.remove('sel');
    hideBtn.setAttribute('aria-checked', 'false');
  }
  const inp = document.getElementById('storeSearch');
  if (inp) inp.value = '';
  await openStore();
  renderStoreItems();
  const grid = document.getElementById('storeGrid');
  if (!grid) return;
  const cards = [...grid.querySelectorAll('.scard[data-store-id]')];
  const card = cards.find(c => c.dataset.storeId === storeId);
  if (!card) return;
  card.scrollIntoView({behavior: 'smooth', block: 'center'});
  card.classList.remove('scard-focus');
  void card.offsetWidth;
  card.classList.add('scard-focus');
  setTimeout(() => card.classList.remove('scard-focus'), 2600);
  const updBtn = card.querySelector('button[data-act="update-store"]');
  if (updBtn) updBtn.focus();
}
async function updateStore(id, btn) {
  const nm = (storeItems && storeItems.find(x => x.id === id) || {}).name || id;
  if (btn) {
    btn.disabled = true;
    btn.innerHTML = '<span class="spin">' + ic('refresh') + '</span> Mise \u00e0 jour\u2026';
  }
  const j = await postAction({action: 'update_store', id}, {quiet: true});
  if (j.ok) {
    if (j.version) {
      if (!storeVersions) storeVersions = {};
      storeVersions[id] = j.version;
    }
    toast(nm + (j.version ? ' ' + fmtStoreVer(j.version) : '') + ' mis \u00e0 jour', 'ok');
    await load();
    renderStoreItems();
  } else {
    if (btn) {
      btn.disabled = false;
      btn.innerHTML = ic('download') + ' Mettre \u00e0 jour';
    }
    toast(j.error || 'erreur', 'err', 4000);
  }
}
async function uninstallStore(id, btn) {
  const nm = (storeItems && storeItems.find(x => x.id === id) || {}).name || id;
  const sid = 'store-' + id;
  const ok = await askConfirm({
    title: 'Supprimer \u00ab ' + nm + ' \u00bb',
    msg: 'Supprimer d\u00e9finitivement \u00ab ' + nm + ' \u00bb et tous ses fichiers ?',
    detail: 'Le processus sera arr\u00eat\u00e9, le serveur sera retir\u00e9 de la configuration et du cache d\u2019\u00e9tat, et les fichiers t\u00e9l\u00e9charg\u00e9s du paquet (npx/uvx) seront supprim\u00e9s du disque.',
    okLabel: 'Supprimer',
    danger: true,
  });
  if (!ok) return;
  if (btn) {
    btn.disabled = true;
    btn.innerHTML = '<span class="spin">' + ic('refresh') + '</span> Suppression\u2026';
  }
  const j = await postAction({action: 'uninstall_store', id}, {quiet: true});
  if (j.ok) {
    if (selectedId === sid) {
      selectedId = null; detailCache = null; _sigOverview = ''; _sigDetail = ''; _lastSelectedSrvSig = '';
    }
    if (Array.isArray(j.warnings) && j.warnings.length) {
      toast(nm + ' supprim\u00e9 (avertissement nettoyage : ' + j.warnings[0] + ')', 'warn', 5000);
    } else {
      toast(nm + ' supprim\u00e9 (configuration et fichiers nettoy\u00e9s)', 'ok');
    }
    await load();
    renderStoreItems();
  } else {
    if (btn) {
      btn.disabled = false;
      btn.innerHTML = ic('trash') + ' Supprimer';
    }
    toast(j.error || 'erreur', 'err', 4000);
  }
}

/* ===== Barre latérale ajustable (largeur mémorisée via data-w, 0 style inline) & repliable (Ctrl+B) ===== */
const snapExplW = w => Math.round(Math.min(520, Math.max(180, w)) / 10) * 10;
function toggleExplorer() {
  const expl = document.getElementById('explorer');
  const btn = document.getElementById('tbExplToggle');
  if (!expl) return;
  const collapsed = expl.classList.toggle('collapsed');
  if (btn) btn.setAttribute('aria-expanded', collapsed ? 'false' : 'true');
  try { localStorage.setItem('mcc.explorer.collapsed', collapsed ? '1' : '0'); } catch (e) {}
}
(function () {
  const grip = document.getElementById('grip'), expl = document.getElementById('explorer');
  if (!grip || !expl) return;
  const setW = w => {
    const snapped = snapExplW(w);
    expl.dataset.w = String(snapped);
    grip.setAttribute('aria-valuenow', String(snapped));
    return snapped;
  };
  try {
    const saved = +localStorage.getItem('mcc.explorer.w');
    if (saved) setW(saved);
    else grip.setAttribute('aria-valuenow', String(snapExplW(expl.dataset.w || expl.getBoundingClientRect().width || 320)));
    if (localStorage.getItem('mcc.explorer.collapsed') === '1') {
      expl.classList.add('collapsed');
      const btn = document.getElementById('tbExplToggle');
      if (btn) btn.setAttribute('aria-expanded', 'false');
    }
  } catch (e) {}
  grip.addEventListener('mousedown', e => {
    e.preventDefault();
    grip.classList.add('drag');
    expl.classList.add('resizing');
    const x0 = e.clientX, w0 = expl.getBoundingClientRect().width;
    const mv = ev => { setW(w0 + ev.clientX - x0); };
    const up = () => {
      try { localStorage.setItem('mcc.explorer.w', String(setW(expl.getBoundingClientRect().width))); } catch (e) {}
      grip.classList.remove('drag');
      expl.classList.remove('resizing');
      document.removeEventListener('mousemove', mv);
      document.removeEventListener('mouseup', up);
      window.removeEventListener('blur', up);
    };
    document.addEventListener('mousemove', mv);
    document.addEventListener('mouseup', up);
    window.addEventListener('blur', up);
  });
  grip.addEventListener('keydown', e => {
    let next = null;
    const cur = snapExplW(+(expl.dataset.w || expl.getBoundingClientRect().width || 320));
    if (e.key === 'ArrowLeft') next = cur - 20;
    else if (e.key === 'ArrowRight') next = cur + 20;
    else if (e.key === 'Home') next = 180;
    else if (e.key === 'End') next = 520;
    if (next === null) return;
    e.preventDefault();
    try { localStorage.setItem('mcc.explorer.w', String(setW(next))); } catch (er) {}
  });
})();

/* ===== Contrôles de fenêtre (desktop Tauri uniquement) ===== */
function tauriInvoke() {
  try {
    const I = window.__TAURI_INTERNALS__;
    if (I && typeof I.invoke === 'function') {
      return cmd => I.invoke(cmd, {});
    }
    const W = window.__TAURI__ && window.__TAURI__.window;
    if (!W) return null;
    const tw = W.getCurrentWindow ? W.getCurrentWindow() : (W.getCurrent ? W.getCurrent() : null);
    if (!tw) return null;
    return cmd => {
      if (cmd === 'plugin:window|start_dragging') return tw.startDragging();
      if (cmd === 'plugin:window|minimize') return tw.minimize();
      if (cmd === 'plugin:window|toggle_maximize') return tw.toggleMaximize();
      if (cmd === 'plugin:window|close') return tw.close();
    };
  } catch (e) { return null; }
}
/* Boutons de fenêtre visibles seulement si l'API fenêtre existe (desktop) :
   hors Tauri ils resteraient des boutons morts. Le drag + double-clic sur
   #titlebar est géré nativement par le script drag.js de Tauri 2 via
   data-tauri-drag-region="deep" (un seul appel IPC, zéro course). */
(function () {
  const inv = tauriInvoke();
  if (!inv) return;
  const box = document.getElementById('winCtrls');
  if (box) box.classList.add('on');
  // Filet de sécurité : écouteur sur WINDOW (phase bulle, donc APRÈS celui de
  // drag.js posé sur document). Si drag.js a pris le clic, il a appelé
  // stopImmediatePropagation → on ne reçoit rien (pas de double appel). S'il
  // ne l'a pas pris (script non injecté sur l'URL loopback, régression
  // Tauri…), on lance nous-mêmes le déplacement.
  window.addEventListener('mousedown', ev => {
    if (ev.button !== 0 || ev.defaultPrevented) return;
    const t = ev.target;
    if (!t || !t.closest || !t.closest('#titlebar')) return;
    if (t.closest('button,a,input,select,textarea,label,[role="menu"],[role="menuitem"],[tabindex]:not([tabindex="-1"]),#winCtrls')) return;
    ev.preventDefault();
    const cmd = ev.detail === 2 ? 'plugin:window|toggle_maximize' : 'plugin:window|start_dragging';
    try { Promise.resolve(inv(cmd)).catch(err => console.warn('[mcc] ' + cmd + ' refusé :', err)); }
    catch (e) { console.warn('[mcc] ' + cmd + ' a levé :', e); }
  });
})();
function winAction(op) {
  const inv = tauriInvoke();
  if (!inv) return;
  const cmds = {
    min: 'plugin:window|minimize',
    max: 'plugin:window|toggle_maximize',
    close: 'plugin:window|close',
  };
  const cmd = cmds[op];
  if (!cmd) return;
  try { Promise.resolve(inv(cmd)).catch(() => {}); } catch (e) { /* API refusée : aucun effet */ }
}

/* Journal replié PAR DÉFAUT (interface épurée) : l'opérateur l'ouvre d'un clic
   sur sa barre et son choix est retenu (localStorage). */
function toggleLog() {
  const lp = document.getElementById('logpanel');
  if (!lp) return;
  const min = lp.classList.toggle('min');
  const btn = document.getElementById('logToggleBtn');
  if (btn) btn.setAttribute('aria-expanded', min ? 'false' : 'true');
  try { localStorage.setItem('mcc.logmin', String(min)); } catch (e) {}
}
(function () {
  const lp = document.getElementById('logpanel');
  if (!lp) return;
  let v = null, exp = null;
  try {
    v = localStorage.getItem('mcc.logmin');
    exp = localStorage.getItem('mcc.logexpanded');
  } catch (e) {}
  const min = v === null ? true : v === 'true';
  lp.classList.toggle('min', min);
  const togBtn = document.getElementById('logToggleBtn');
  if (togBtn) togBtn.setAttribute('aria-expanded', min ? 'false' : 'true');
  const expanded = exp === 'true';
  lp.classList.toggle('expanded', expanded);
  const expBtn = document.getElementById('logExpandBtn');
  if (expBtn) {
    expBtn.classList.toggle('sel', expanded);
    expBtn.setAttribute('aria-pressed', expanded ? 'true' : 'false');
  }
})();
/* Helpers pour les onglets Identité & Sessions, Guide d'appairage et Verdicts
   (toutes les interpolations passent par esc() / ic()). */
function copyDataText(el) {
  const txt = el && el.dataset ? el.dataset.text : '';
  if (txt) copyText(txt);
}
function pairGuideHtml(uiUrl) {
  const url = uiUrl || (window.location && window.location.origin) || 'http://127.0.0.1:5090';
  const bCmd = (state && state.bridge_command) || 'mcp-control';
  const cmdPair = quoteShellArg(bCmd) + ' auth pair ' + url;
  const jsonBridge = JSON.stringify({
    mcpServers: {
      'mcp-control': {
        command: bCmd,
        args: ['bridge', '--url', url]
      }
    }
  }, null, 2);
  const blk = (lang, title, hl) =>
    `<div class="conf-block"><div class="conf-head"><span class="conf-lang">${esc(lang)}</span><span>${esc(title)}</span><button type="button" class="copy-btn" data-act="copy-conf">${ic('copy')}<span>Copier</span></button></div><pre>${hl}</pre></div>`;
  return `<div class="acc-sub">Connecter un client / assistant IA</div>
    <div class="acc-card acc-guide-card">
      <div class="acc-who"><b>1. Appairer la machine cliente</b> &mdash; ex&eacute;cutez cette commande puis validez le code &agrave; 6 chiffres ci-dessus :</div>
      ${blk('CLI', 'Terminal — Demande d\u2019appairage Ed25519', hlCli(cmdPair))}
      <div class="acc-who"><b>2. Configurer Hermes / Grokbot / Claude Code / Cursor</b> &mdash; le pont local signe chaque requ&ecirc;te RFC 9421 :</div>
      ${blk('JSON', 'JSON mcpServers — Pont sign\u00e9 mcp-control bridge', hlJson(jsonBridge))}
    </div>`;
}
function renderAccIdentity() {
  const box = document.getElementById('accIdentity');
  if (!box || !accessState) return;
  const id = accessState.server_identity || {};
  const tls = accessState.tls || {};
  const sess = accessState.sessions || {active: (state && state.sessions_active) || 1, max: 8};
  const storageBadge = id.storage === 'keyring'
    ? `<span class="acc-tag ok">${ic('check')} Trousseau syst&egrave;me (keyring)</span>`
    : (id.storage === 'file_fallback'
      ? `<span class="acc-tag warn">${ic('warn')} Repli fichier local (file_fallback)</span>`
      : `<span class="acc-tag bad">${ic('x')} Indisponible (${esc(id.storage || 'unavailable')})</span>`);
  const storageWarn = id.storage === 'file_fallback'
    ? `<div class="acc-who tone-warn">${ic('warn')} La cl&eacute; priv&eacute;e du serveur est stock&eacute;e en fichier local (trousseau syst&egrave;me indisponible ou d&eacute;sactiv&eacute;).</div>`
    : (id.storage && id.storage !== 'keyring'
      ? `<div class="acc-who tone-bad">${ic('warn')} Identit&eacute; cryptographique du serveur non disponible.</div>`
      : '');
  const tlsBadge = tls.bind_refused
    ? `<span class="acc-tag bad">${ic('x')} Bind LAN refus&eacute; (sans TLS)</span>`
    : (tls.active
      ? `<span class="acc-tag ok">${ic('lock')} TLS 1.3 actif</span>`
      : (tls.compiled
        ? `<span class="acc-tag">${ic('shield')} Loopback HTTP (TLS 1.3 compil&eacute;)</span>`
        : `<span class="acc-tag warn">Non compil&eacute; (loopback seul)</span>`));
  const verifyCmd = 'mcp-control auth verify-log';
  box.innerHTML = `
    <div class="acc-card">
      <div class="acc-h">
        <span class="acc-tool">Identit&eacute; du serveur (${esc(id.keyid || 'mcc-server')})</span>
        ${storageBadge}
      </div>
      ${storageWarn}
      <div class="acc-kv"><span class="acc-k">keyid</span><span class="acc-v">${esc(id.keyid || 'mcc-server')}</span></div>
      <div class="acc-kv"><span class="acc-k">empreinte courte</span><span class="acc-v">${esc(id.fingerprint_court || '—')}</span></div>
      <div class="acc-kv"><span class="acc-k">empreinte compl&egrave;te</span><span class="acc-v">${esc(id.fingerprint || '—')}</span></div>
      ${id.fingerprint ? `<div class="acc-acts"><button data-act="copy-text" data-text="${esc(id.fingerprint)}">${ic('copy')} Copier l'empreinte serveur</button></div>` : ''}
    </div>
    <div class="acc-card">
      <div class="acc-h">
        <span class="acc-tool">R&eacute;seau &amp; TLS 1.3</span>
        ${tlsBadge}
      </div>
      <div class="acc-kv"><span class="acc-k">URL d'acc&egrave;s</span><span class="acc-v">${esc(accessState.ui_url || '—')}</span></div>
      <div class="acc-kv"><span class="acc-k">certificat TLS</span><span class="acc-v">${esc(tls.fingerprint || 'aucun (loopback)')}</span></div>
      ${tls.notice ? `<div class="acc-who">${esc(tls.notice)}</div>` : ''}
      ${tls.fingerprint ? `<div class="acc-acts"><button data-act="copy-text" data-text="${esc(tls.fingerprint)}">${ic('copy')} Copier l'empreinte TLS</button></div>` : ''}
    </div>
    <div class="acc-card">
      <div class="acc-h">
        <span class="acc-tool">Sessions locales</span>
        <span class="acc-tag ok">${esc(sess.active ?? 1)} / ${esc(sess.max ?? 8)} sessions actives</span>
      </div>
      <div class="acc-who">Chaque onglet local authentifi&eacute; d&eacute;tient un cookie HttpOnly <code>mcc_session</code> coupl&eacute; &agrave; un secret en m&eacute;moire (<code>X-MCC-Session</code>).</div>
      <div class="acc-acts">
        <button class="danger" data-act="close-other-sessions">${ic('lock')} Fermer les autres sessions</button>
      </div>
    </div>
    <div class="acc-card">
      <div class="acc-h">
        <span class="acc-tool">Journal d'audit cha&icirc;n&eacute; (HMAC-SHA256)</span>
        <span class="acc-tag">${ic('shield')} auth-audit.log</span>
      </div>
      <div class="acc-who">Chaque d&eacute;cision d'authentification et d'autorisation sur <code>/mcp</code> est scell&eacute;e par un code HMAC-SHA256 cha&icirc;n&eacute; &agrave; l'entr&eacute;e pr&eacute;c&eacute;dente. V&eacute;rifiez l'int&eacute;grit&eacute; de la cha&icirc;ne avec :</div>
      <div class="conf-block"><div class="conf-head"><span class="conf-lang">CLI</span><span>V&eacute;rification d'int&eacute;grit&eacute; du journal</span><button type="button" class="copy-btn" data-act="copy-conf">${ic('copy')}<span>Copier</span></button></div><pre>${hlCli(verifyCmd)}</pre></div>
    </div>`;
}
async function closeOtherSessions() {
  const ok = await askConfirm({
    title: 'Fermer les autres sessions',
    msg: 'D\u00e9connecter toutes les autres sessions locales actives ? Cette fen\u00eatre restera connect\u00e9e.',
    okLabel: 'Fermer les autres',
    danger: true,
  });
  if (!ok) return;
  try {
    const r = await fetch('/api/session/close-others', {
      method: 'POST',
      headers: {'Content-Type': 'application/json'},
      body: '{}'
    });
    const j = await r.json().catch(() => null);
    if (r.ok && j && j.ok) {
      const n = j.closed || 0;
      toast(n === 0
        ? 'Aucune autre session active'
        : (n + ' autre' + (n > 1 ? 's' : '') + ' session' + (n > 1 ? 's' : '') + ' ferm\u00e9e' + (n > 1 ? 's' : '')), 'ok');
      await refreshAccess();
      await load();
    } else {
      accessMsg('\u2717 ' + ((j && j.error) || 'impossible de fermer les autres sessions'));
    }
  } catch (e) {
    accessMsg('\u2717 erreur r\u00e9seau lors de la fermeture des sessions');
  }
}
function verdictTone(v) {
  const s = String(v || '').toLowerCase();
  if (s.includes('allow') || s.includes('autoris') || s === 'ok' || s.includes('approv')) return 'ok';
  if (s.includes('expir')) return 'warn';
  return 'bad';
}
function verdictIcon(v) {
  const s = String(v || '').toLowerCase();
  if (s.includes('allow') || s.includes('autoris') || s === 'ok' || s.includes('approv')) return 'check';
  if (s.includes('expir')) return 'warn';
  if (s.includes('revo')) return 'lock';
  return 'x';
}
/* Ticker local 1000 ms pour les comptes à rebours de #accessDlg : met uniquement
   à jour .textContent des éléments [data-ttl-until] sans requête réseau ni innerHTML. */
let _accessZeroSig = '';
function tickAccessTtl() {
  const dlg = document.getElementById('accessDlg');
  if (!dlg || !dlg.open) return;
  const now = Date.now();
  let expiredNow = false;
  dlg.querySelectorAll('[data-ttl-until]').forEach(el => {
    const until = +el.dataset.ttlUntil || 0;
    const rem = Math.max(0, Math.ceil((until - now) / 1000));
    const txt = el.dataset.ttlMode === 'grant'
      ? (rem + ' s restantes')
      : ('expire dans ' + rem + ' s');
    setText(el, txt);
    if (rem === 0 && until > 0 && !el.dataset.ttlExpired) {
      el.dataset.ttlExpired = '1';
      expiredNow = true;
    }
  });
  if (expiredNow && _accessZeroSig !== _sigAccess) {
    _accessZeroSig = _sigAccess;
    refreshAccess();
  }
}

/* ===== Écran « Accès » (lot M3, docs/auth.md §5) ========================
   Cinq volets : confirmations en attente (ce qui va être lancé), appairage
   (demandes + guide), clients appairés (droits, inactivité > 180 j, rotation,
   révocation), identité & sessions, 100 dernières décisions. TOUT texte venu
   d'un client passe par esc() : un libellé hostile ne doit jamais devenir du
   HTML (test « libellé hostile » côté Rust). Les ordres partent par postAction
   (session locale) — jamais un fetch direct vers /api/action. */
let accessState = null;
let accessTab = 'confirmations';
let accessTimer = null;
let clientFilterMode = 'all';
let decisionFilterQuery = '';
const pairDroits = {}; // pair_id -> droits choisis localement (défaut : lecture)

function accessMsg(txt) {
  const el = document.getElementById('accessMsg');
  if (el) el.textContent = txt || '';
}
function accessBadge() {
  const a = (state && state.access) || {};
  const n = (a.pending || 0) + (a.pairs || 0);
  const b = document.getElementById('accessBanner');
  const btn = document.getElementById('accessBannerBtn');
  if (b) {
    setHidden(b, !n);
    if (n && btn) {
      const parts = [];
      if (a.pending) parts.push(a.pending + ' confirmation' + (a.pending > 1 ? 's' : '') + ' avant ex\u00e9cution');
      if (a.pairs) parts.push(a.pairs + ' demande' + (a.pairs > 1 ? 's' : '') + ' d\u2019appairage');
      const html = ic('shield') + '<span>' + parts.join(' et ') + ' en attente &mdash; cliquer pour d\u00e9cider</span>';
      if (btn.innerHTML !== html) btn.innerHTML = html;
    }
  }
  const badge = document.getElementById('tbAccessN');
  if (badge) { setHidden(badge, !n); setText(badge, n); }
  const cp = document.getElementById('accCountPending');
  if (cp) { setHidden(cp, !(a.pending || 0)); setText(cp, a.pending || 0); }
  const cpa = document.getElementById('accCountPairs');
  if (cpa) { setHidden(cpa, !(a.pairs || 0)); setText(cpa, a.pairs || 0); }
}
function openAccessDlg(pending) {
  const d = document.getElementById('accessDlg');
  if (!d) return;
  if (!d.open) { rememberFocus(); d.showModal(); }
  let tab = 'confirmations';
  if (!pending) { try { tab = localStorage.getItem('mcc.acctab') || tab; } catch (e) {} }
  setAccessTab(tab);
  accessMsg('');
  const nav = d.querySelector('.set-nav-btn.sel');
  if (nav) nav.focus();
  refreshAccess(true);
  if (accessTimer) clearInterval(accessTimer);
  accessTimer = setInterval(tickAccessTtl, 1000);
}
function closeAccessDlg() {
  if (accessTimer) { clearInterval(accessTimer); accessTimer = null; }
  const d = document.getElementById('accessDlg');
  if (d && d.open) d.close();
  restoreFocus();
}
function setAccessTab(name) {
  accessTab = name;
  document.querySelectorAll('#accessDlg .set-nav-btn').forEach(b => {
    const sel = b.dataset.atab === name;
    b.classList.toggle('sel', sel);
    b.setAttribute('aria-selected', sel ? 'true' : 'false');
    b.setAttribute('tabindex', sel ? '0' : '-1');
  });
  document.querySelectorAll('#accessDlg .set-pane').forEach(p => p.classList.toggle('sel', p.dataset.atab === name));
  try { localStorage.setItem('mcc.acctab', name); } catch (e) {}
}
async function refreshAccess(forceClear) {
  const j = await postAction({action: 'access_list'}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'lecture impossible')); return; }
  accessState = j;
  if (forceClear) {
    accessMsg('');
  } else {
    const el = document.getElementById('accessMsg');
    if (el && el.textContent && el.textContent.includes('lecture')) accessMsg('');
  }
  renderAccess();
}
function renderAccess() {
  if (!accessState) return;
  const sig = JSON.stringify([accessState, clientFilterMode, decisionFilterQuery, pairDroits]);
  if (sig === _sigAccess) return;
  _sigAccess = sig;
  renderAccPending();
  renderAccClients();
  renderAccPairs();
  renderAccIdentity();
  renderAccDecisions();
  accessBadge();
}
/* Effet exact d'un appel : tout est échappé, rien n'est interprété. */
function effectHtml(e) {
  if (!e || typeof e !== 'object') return '';
  const out = [];
  const add = (k, v) => {
    const t = v === undefined || v === null ? '' : String(v);
    if (t) out.push('<div class="acc-kv"><span class="acc-k">' + esc(k) + '</span><span class="acc-v">' + esc(t) + '</span></div>');
  };
  if (e.kind === 'lancement') {
    add('programme', e.program);
    if (Array.isArray(e.args) && e.args.length) add('arguments', e.args.join(' '));
    add('dossier de travail', e.workdir);
    if (Array.isArray(e.env) && e.env.length) add('variables', e.env.join(', ') + ' (valeurs masquées)');
  } else if (e.kind === 'outil') {
    add('serveur', e.target);
    add('outil', e.tool);
    if (e.arguments !== undefined) add('arguments', JSON.stringify(e.arguments));
  } else if (e.kind === 'ssh') {
    add('cible', e.target);
  } else if (e.kind === 'import') {
    (e.servers || []).forEach((s, i) => add('serveur ' + (i + 1),
      [s.label, s.program].filter(Boolean).join(' \u2014 ') +
      (Array.isArray(s.args) && s.args.length ? ' ' + s.args.join(' ') : '')));
  } else {
    add('cible', e.target || e.label);
  }
  if (e.note) add('note', e.note);
  return out.join('');
}
function renderAccPending() {
  const box = document.getElementById('accPending');
  const grantsBox = document.getElementById('accGrants');
  if (!box || !grantsBox) return;
  const nowMs = Date.now();
  const pend = accessState.pending || [];
  const ttl = accessState.request_ttl_secs || 120;
  let html = pend.length ? '' :
    '<div class="empty-sec">aucune confirmation en attente \u2014 un appel d\u2019outil ' +
    '<code>execution</code> appara\u00eetra ici et rien ne sera lanc\u00e9 sans votre accord</div>';
  html += pend.map(p => `<div class="acc-card warn">
    <div class="acc-h">
      <span class="acc-tool">${esc(p.tool)}</span>
      <span class="acc-ttl" data-ttl-until="${esc(nowMs + Number(p.expires_in || 0) * 1000)}" data-ttl-mode="pending" title="une demande non d\u00e9cid\u00e9e expire au bout de ${esc(ttl)} s">expire dans ${esc(p.expires_in)} s</span>
    </div>
    <div class="acc-who">${esc(p.label || p.keyid)} <span class="acc-fp" title="${esc(p.fingerprint)}">${esc(String(p.fingerprint || '').slice(0, 24))}</span> depuis <b>${esc(p.address)}</b></div>
    <div class="acc-what">${esc((p.effect || {}).resume || '')}</div>
    ${effectHtml(p.effect)}
    <div class="acc-acts">
      <button class="primary" data-act="confirm-approve" data-id="${esc(p.id)}" data-mode="once">Accepter une fois</button>
      <button class="primary" data-act="confirm-approve" data-id="${esc(p.id)}" data-mode="window">Accepter ${esc(Math.round((accessState.window_secs || 600) / 60))} minutes</button>
      <button class="danger" data-act="confirm-deny" data-id="${esc(p.id)}">Refuser</button>
    </div>
  </div>`).join('');
  box.innerHTML = html;

  const grants = accessState.grants || [];
  let gh = '<div class="acc-sub">Permis en cours</div>';
  gh += grants.length ? '' : '<div class="empty-sec">aucun permis actif</div>';
  gh += grants.map(g => `<div class="acc-card ok">
    <div class="acc-h">
      <span class="acc-tool">${esc(g.tool)}</span>
      <span class="acc-tag">${esc(g.mode === 'once' ? 'une fois' : 'fen\u00eatre')}</span>
      <span class="acc-ttl" data-ttl-until="${esc(nowMs + Number(g.expires_in || 0) * 1000)}" data-ttl-mode="grant">${esc(g.expires_in)} s restantes</span>
    </div>
    <div class="acc-who">${esc(g.label || g.keyid)}</div>
    <div class="acc-acts"><button class="danger" data-act="grant-revoke" data-id="${esc(g.id)}">R\u00e9voquer</button></div>
  </div>`).join('');
  grantsBox.innerHTML = gh;
}
function setClientFilter(mode) {
  clientFilterMode = mode || 'all';
  renderAccClients();
}
function renderAccClients() {
  const box = document.getElementById('accClients');
  if (!box) return;
  const focused = document.activeElement && box.contains(document.activeElement) ? document.activeElement : null;
  const fSpec = focused && focused.dataset ? {
    act: focused.dataset.act || '',
    mode: focused.dataset.mode || '',
    id: focused.dataset.id || '',
    right: focused.dataset.right || '',
  } : null;
  const allClients = accessState.clients || [];
  const clients = allClients.filter(c => {
    if (clientFilterMode === 'active') return c.status !== 'revoked';
    if (clientFilterMode === 'revoked') return c.status === 'revoked';
    return true;
  });
  let html = '<div class="acc-toolbar" role="group" aria-label="Filtrer les clients">' +
    '<button type="button" class="exp-pill' + (clientFilterMode === 'all' ? ' sel' : '') + '" aria-pressed="' + (clientFilterMode === 'all' ? 'true' : 'false') + '" data-act="client-filter" data-mode="all">Tous (' + esc(allClients.length) + ')</button>' +
    '<button type="button" class="exp-pill' + (clientFilterMode === 'active' ? ' sel' : '') + '" aria-pressed="' + (clientFilterMode === 'active' ? 'true' : 'false') + '" data-act="client-filter" data-mode="active">Actifs (' + esc(allClients.filter(c => c.status !== 'revoked').length) + ')</button>' +
    '<button type="button" class="exp-pill' + (clientFilterMode === 'revoked' ? ' sel' : '') + '" aria-pressed="' + (clientFilterMode === 'revoked' ? 'true' : 'false') + '" data-act="client-filter" data-mode="revoked">R\u00e9voqu\u00e9s (' + esc(allClients.filter(c => c.status === 'revoked').length) + ')</button>' +
    '</div>';
  html += clients.length ? '' : '<div class="empty-sec">aucun client appair\u00e9 dans cette vue</div>';
  html += clients.map(c => {
    const droits = c.rights || [];
    const boutons = ['lecture', 'pilotage', 'execution'].map(r => {
      const classeOn = droits.includes(r) ? ' on' : '';
      return `<button type="button" class="acc-right${classeOn}" role="switch" aria-checked="${esc(droits.includes(r) ? 'true' : 'false')}" data-act="client-right" data-id="${esc(c.keyid)}" data-right="${esc(r)}"
        title="${esc(droits.includes(r) ? 'Retirer' : 'Accorder')} le droit ${esc(r)}">${esc(r)}</button>`;
    }).join('');
    const revoque = c.status === 'revoked';
    const classeOff = revoque ? ' off' : '';
    const badgeRevoque = (revoque ? '<span class="acc-tag bad">' + esc('révoqué') + '</span>' : '<span class="acc-tag ok">' + esc('actif') + '</span>')
      + (c.stale_180d ? ' <span class="acc-tag warn" title="Aucune utilisation depuis plus de 180 jours">Inactif &gt; 180 j</span>' : '');
    const btnRevoque = revoque ? ''
      : '<span class="acc-acts-sep" aria-hidden="true"></span>'
        + '<button type="button" data-act="client-rotate" data-id="' + esc(c.keyid) + '">Rotation de cl\u00e9</button>'
        + '<button type="button" class="danger" data-act="client-revoke" data-id="' + esc(c.keyid) + '">R\u00e9voquer</button>';
    return `<div class="acc-card${classeOff}">
      <div class="acc-h">
        <span class="acc-tool">${esc(c.label || c.keyid)}</span>
        <span class="acc-tag">${esc(c.keyid)}</span>
        ${badgeRevoque}
      </div>
      <div class="acc-who"><span class="acc-fp" title="${esc(c.fingerprint)}">${esc(c.fingerprint_court || c.fingerprint)}</span>
        \u00b7 appair\u00e9 depuis ${esc(c.paired_from || '?')}
        \u00b7 cr\u00e9\u00e9 le ${esc(c.created_at || '?')}
        \u00b7 derni\u00e8re utilisation : ${esc(c.last_used_at || 'jamais')}</div>
      <div class="acc-acts">${boutons}${btnRevoque}
      </div>
    </div>`;
  }).join('');
  box.innerHTML = html;
  if (fSpec && fSpec.act) {
    const target = [...box.querySelectorAll('button[data-act]')].find(b =>
      b.dataset.act === fSpec.act
      && (b.dataset.mode || '') === fSpec.mode
      && (b.dataset.id || '') === fSpec.id
      && (b.dataset.right || '') === fSpec.right
    );
    if (target) target.focus();
  }
}

function renderAccPairs() {
  const box = document.getElementById('accPairs');
  if (!box) return;
  const nowMs = Date.now();
  const pairs = accessState.pairs || [];
  let html = pairs.length ? '' :
    '<div class="empty-sec">aucune demande d\u2019appairage en attente \u2014 un client qui lance <code>mcp-control auth pair</code> appara\u00eetra ici</div>';
  html += pairs.map(p => {
    if (!pairDroits[p.pair_id]) pairDroits[p.pair_id] = ['lecture'];
    const choisis = pairDroits[p.pair_id];
    const cases = ['lecture', 'pilotage', 'execution'].map(r =>
      `<label class="acc-chk"><input type="checkbox" data-pair="${esc(p.pair_id)}" data-right="${esc(r)}"
        ${esc(choisis.includes(r) ? 'checked' : '')} data-act="pair-right">${esc(r)}</label>`).join('');
    return `<div class="acc-card">
      <div class="acc-h">
        <span class="acc-code" title="le client affiche le m\u00eame code : s\u2019ils diff\u00e8rent, refusez">${esc(p.code)}</span>
        <span class="acc-ttl" data-ttl-until="${esc(nowMs + Number(p.expires_in || 0) * 1000)}" data-ttl-mode="pending">expire dans ${esc(p.expires_in)} s</span>
      </div>
      <div class="acc-who">${esc(p.label || p.keyid)} <span class="acc-fp" title="${esc(p.fingerprint)}">${esc(p.fingerprint_court || '')}</span>
        \u00b7 ${esc(p.keyid)} \u00b7 depuis ${esc(p.paired_from || '?')}</div>
      <div class="acc-acts">${cases}<span class="acc-acts-sep" aria-hidden="true"></span>
        <button type="button" class="primary" data-act="pair-approve" data-id="${esc(p.pair_id)}">Accepter</button>
        <button type="button" class="danger" data-act="pair-reject" data-id="${esc(p.pair_id)}">Refuser</button>
      </div>
    </div>`;
  }).join('');
  html += pairGuideHtml(accessState.ui_url);
  box.innerHTML = html;
}
function setDecisionFilter(el) {
  decisionFilterQuery = (el && el.value ? el.value : '').trim().toLowerCase();
  renderAccDecisionsList();
}
function renderAccDecisions() {
  const box = document.getElementById('accDecisions');
  if (!box) return;
  let listBox = document.getElementById('accDecisionsRows');
  if (!listBox) {
    box.innerHTML = '<div class="acc-toolbar">' +
      '<label for="accDecisionFilter" class="sr-only">Filtrer le journal d\u2019audit</label>' +
      '<input id="accDecisionFilter" placeholder="Filtrer par outil, client, verdict ou motif\u2026" data-act="decision-filter" value="' + esc(decisionFilterQuery) + '">' +
      '</div><div id="accDecisionsRows"></div>';
  }
  renderAccDecisionsList();
}
function renderAccDecisionsList() {
  const rowsBox = document.getElementById('accDecisionsRows');
  if (!rowsBox || !accessState) return;
  const all = accessState.decisions || [];
  const q = decisionFilterQuery;
  const d = q ? all.filter(x => {
    const hay = ((x.verdict || '') + ' ' + (x.tool || '') + ' ' + (x.keyid || '') + ' ' + (x.reason || '')).toLowerCase();
    return hay.includes(q);
  }) : all;
  const heure = ts => {
    try { return new Date(Number(ts) * 1000).toLocaleTimeString(); } catch (e) { return '?'; }
  };
  let html = d.length ? '' : '<div class="empty-sec">aucune d\u00e9cision enregistr\u00e9e pour ce filtre</div>';
  html += d.map(x => `<div class="acc-line">
    <span class="acc-when">${esc(heure(x.ts))}</span>
    <span class="acc-verdict"><span class="acc-tag ${esc(verdictTone(x.verdict))}">${ic(verdictIcon(x.verdict))} ${esc(x.verdict)}</span></span>
    <span class="acc-tool">${esc(x.tool)}</span>
    <span class="acc-who-sm">${esc(x.keyid)}</span>
    <span class="acc-reason">${esc(x.reason === '-' ? '' : (x.reason || ''))}</span>
  </div>`).join('');
  rowsBox.innerHTML = html;
}
async function confirmApprove(id, mode) {
  const j = await postAction({action: 'confirm_approve', request_id: id, mode}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  toast(mode === 'once' ? 'confirm\u00e9 : une seule ex\u00e9cution' : 'confirm\u00e9 pour la dur\u00e9e du permis', 'ok');
  await refreshAccess(true);
  await load();
}
async function confirmDeny(id) {
  const j = await postAction({action: 'confirm_deny', request_id: id}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  toast('demande refus\u00e9e : rien ne sera ex\u00e9cut\u00e9', 'ok');
  await refreshAccess(true);
  await load();
}
async function grantRevoke(id) {
  const j = await postAction({action: 'grant_revoke', request_id: id}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  await refreshAccess(true);
}
async function clientToggleRight(keyid, right) {
  const c = (accessState.clients || []).find(x => x.keyid === keyid);
  if (!c) return;
  const actuels = c.rights || [];
  const droits = actuels.includes(right) ? actuels.filter(r => r !== right) : actuels.concat([right]);
  const j = await postAction({action: 'client_set_scopes', keyid, rights: droits}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  await refreshAccess(true);
}
async function clientRotateModal(keyid) {
  const res = await askConfirm({
    title: 'Rotation de cl\u00e9 Ed25519',
    msg: 'Remplacer la cl\u00e9 publique du client ' + keyid + ' en conservant ses droits :',
    inputs: [
      {id: 'new_keyid', label: 'Nouveau keyid (mcc-<nom>-<16hex>)', placeholder: 'mcc-cli-0123456789abcdef'},
      {id: 'public_key', label: 'Nouvelle cl\u00e9 publique Ed25519 (64 hex)', placeholder: '64 caract\u00e8res hexad\u00e9cimaux minuscules'}
    ],
    okLabel: 'Effectuer la rotation',
  });
  if (!res || typeof res !== 'object') return;
  if (!res.new_keyid || !res.public_key) {
    accessMsg('\u2717 nouveau keyid et cl\u00e9 publique requis');
    return;
  }
  const j = await postAction({action: 'client_rotate', keyid, new_keyid: res.new_keyid, public_key: res.public_key}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'rotation refus\u00e9e')); return; }
  toast('rotation effectu\u00e9e : ' + (j.keyid || res.new_keyid), 'ok');
  await refreshAccess(true);
}
async function clientRevokeConfirm(keyid) {
  const ok = await askConfirm({
    title: 'R\u00e9voquer ce client',
    msg: 'R\u00e9voquer ' + keyid + ' ? Ses requ\u00eates seront refus\u00e9es d\u00e8s la suivante.',
    okLabel: 'R\u00e9voquer',
    danger: true,
  });
  if (!ok) return;
  const j = await postAction({action: 'client_revoke', keyid}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  await refreshAccess(true);
}
function pairRightChange(el) {
  const id = el.dataset.pair;
  const droit = el.dataset.right;
  const liste = pairDroits[id] || [];
  if (el.checked) { if (!liste.includes(droit)) liste.push(droit); } else {
    const i = liste.indexOf(droit);
    if (i >= 0) liste.splice(i, 1);
  }
  pairDroits[id] = liste;
}
async function pairApprove(pairId) {
  const droits = pairDroits[pairId] || ['lecture'];
  const j = await postAction({action: 'pair_approve', pair_id: pairId, rights: droits}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  delete pairDroits[pairId];
  toast('client appair\u00e9', 'ok');
  await refreshAccess(true);
  await load();
}
async function pairReject(pairId) {
  const j = await postAction({action: 'pair_reject', pair_id: pairId}, {quiet: true});
  if (!j.ok) { accessMsg('\u2717 ' + (j.error || 'refus')); return; }
  delete pairDroits[pairId];
  await refreshAccess(true);
}
/* Esc ou clic hors contenu ferme l'écran Accès (aucune saisie en cours : rien à jeter). */
(function () {
  const d = document.getElementById('accessDlg');
  if (!d) return;
  d.addEventListener('cancel', (e) => { e.preventDefault(); closeAccessDlg(); });
  d.addEventListener('click', (e) => { if (e.target === d) closeAccessDlg(); });
})();

/* ===== Polling armé sur les réglages =====
   Rythme normal : settings.refresh_secs (min 5 s), ré-armé à chaque état
   reçu. Panne moteur (lot C #2) : après le PREMIER échec de load(), réessai
   RAPIDE à 2 s (POLL_RETRY_MS) au lieu du prochain cycle complet — bandeau
   toujours au 2ᵉ échec, mais ~refresh_secs+2 s au lieu de ~2×refresh_secs ;
   pendant la panne, chaque échec réarme un réessai de 2 s (fourchette mission
   2 à 5 s) ; dès que le moteur répond, render() → armPolling() revient au
   rythme normal. Un seul timer à la fois : scheduleNext remplace le
   setTimeout en attente (fenêtre masquée : load() sort tôt, la chaîne reprend
   au retour par visibilitychange). */
let pollTimer = null;
const POLL_RETRY_MS = 2000; // panne : réessai rapide (lot C #2)
const POLL_UPDATE_MS = 350; // suivi temps réel pendant une mise à jour
function scheduleNext(ms) {
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = setTimeout(load, ms);
}
function armPolling() {
  if (_updatingNow || _updateRestarting || (state && state.update_status && state.update_status.active)) {
    scheduleNext(POLL_UPDATE_MS);
    return;
  }
  const secs = Math.max(5, (state && state.settings && state.settings.refresh_secs) || 5);
  scheduleNext(secs * 1000);
}
/* ===== Table action → fonction (lot S2) ==================================
   AUCUN attribut on* nulle part : chaque contrôle porte data-act (clé de
   cette table) et, au besoin, data-id / data-op / data-on / data-tab /
   data-mode / data-cat / data-key / data-tool / data-pair / data-right /
   data-i. Les valeurs data-* sont des CHAÎNES relues dans dataset — jamais
   évaluées, jamais concaténées dans du code : une valeur venue du moteur ne
   peut pas devenir du JavaScript. Tout texte affiché passe par esc() au
   moment où le HTML est écrit. */
/* ===== Menu « Découverte » (barre de titre : ports, réseau, machines) ===== */
function closeDiscoverMenu() {
  const menu = document.getElementById('discoverMenu');
  const btn = document.getElementById('tbDiscover');
  if (menu) menu.hidden = true;
  if (btn) btn.setAttribute('aria-expanded', 'false');
}
function toggleDiscoverMenu() {
  const menu = document.getElementById('discoverMenu');
  const btn = document.getElementById('tbDiscover');
  if (!menu || !btn) return;
  const open = menu.hidden;
  menu.hidden = !open;
  btn.setAttribute('aria-expanded', open ? 'true' : 'false');
  if (open) {
    const first = menu.querySelector('.tb-menu-item');
    if (first) first.focus();
  }
}
document.addEventListener('click', ev => {
  const menu = document.getElementById('discoverMenu');
  if (!menu || menu.hidden) return;
  const wrap = document.getElementById('tbDiscoverWrap');
  if (wrap && !wrap.contains(ev.target)) closeDiscoverMenu();
});
document.addEventListener('focusout', ev => {
  const menu = document.getElementById('discoverMenu');
  if (!menu || menu.hidden) return;
  const wrap = document.getElementById('tbDiscoverWrap');
  if (wrap && ev.relatedTarget && !wrap.contains(ev.relatedTarget)) closeDiscoverMenu();
});
document.addEventListener('keydown', ev => {
  if (ev.key !== 'Escape') return;
  const menu = document.getElementById('discoverMenu');
  if (menu && !menu.hidden) {
    closeDiscoverMenu();
    const btn = document.getElementById('tbDiscover');
    if (btn) btn.focus();
  }
});

const ACT = {
  // -- barre de titre, bandeaux, journal
  'toggle-explorer': () => toggleExplorer(),
  'logout': () => logout(),
  'discover-toggle': () => toggleDiscoverMenu(),
  'scan': () => { closeDiscoverMenu(); doScan(); },
  'scan-net': () => { closeDiscoverMenu(); doScanNet(); },
  'machines': () => { closeDiscoverMenu(); openMachineDlg(); },
  'expl-tab': el => setExplorerTab(el.dataset.tab),
  'swarm-open': () => openSwarmView(),
  'swarm-refresh': () => swarmRefresh(true),
  'swarm-sync-logos': () => swarmSyncLogos(),
  'swarm-live-toggle': () => swarmToggleLive(),
  'swarm-a2a-filter': el => swarmSetA2aFilter(el.dataset.mode),
  'swarm-a2a-send': () => swarmSendA2a(),
  'swarm-tab': el => swarmSetTab(el.dataset.tab),
  'swarm-open-console': el => swarmOpenConsole(el.dataset.target, el.dataset.agent, el.dataset.inst),
  'swarm-inst-change': el => swarmInstChange(el),
  'swarm-filter-change': () => swarmFilterChange(),
  'swarm-toggle-create': () => swarmToggleCreate(),
  'swarm-quick-create': () => swarmQuickCreate(),
  'swarm-new-cls': el => swarmNewClassChange(el),
  'swarm-create-submit': () => swarmCreateSubmit(),
  'swarm-agent-start': el => swarmAgentStart(el.dataset.inst, el.dataset.name, el.dataset.runner),
  'swarm-agent-stop': el => swarmAgentStop(el.dataset.inst, el.dataset.name, el.dataset.runner),
  'swarm-agent-delete': el => swarmAgentDelete(el.dataset.inst, el.dataset.name, el.dataset.runner),
  'swarm-res-status': el => swarmResStatus(el.dataset.id),
  'swarm-res-stop': el => swarmResStop(el.dataset.id, el.dataset.clear === '1'),
  'swarm-bridge-op': el => swarmBridgeOp(el.dataset.port, el.dataset.op),
  'swarm-console-target': el => swarmConsoleTargetChange(el),
  'swarm-send-ask': () => swarmSendAsk(),
  'swarm-clear-feed': () => swarmClearFeed(),
  'swarm-load-cfg': el => swarmLoadConfig(el.dataset.inst, el.dataset.name, el.dataset.runner),
  'swarm-cfg-pick': el => swarmCfgPick(el),
  'swarm-cfg-file': el => swarmCfgFileSwitch(el.dataset.file),
  'swarm-cfg-save': () => swarmCfgSave(),
  'swarm-cfg-wire-mcp': () => swarmCfgWireMcp(),
  'swarm-par-add': () => swarmParAdd(),
  'swarm-par-del': el => swarmParDel(el.dataset.idx),
  'swarm-par-run': () => swarmParRun(),
  'swarm-run-ctrl': el => swarmRunCtrl(el.dataset.inst, el.dataset.op, el.dataset.runid, el.dataset.agent),
  'swarm-stream-close': () => swarmStreamClose(),
  'store': () => openStore(),
  'access-open': el => openAccessDlg(el.dataset.pending === '1'),
  'settings-open': () => openSettingsDlg(),
  'refresh-all': () => refreshAll(),
  'log-toggle': () => toggleLog(),
  'log-filter': el => setLogFilter(el.dataset.level),
  'log-copy': () => copyVisibleLog(),
  'log-clear': () => clearVisibleLog(),
  'log-expand': () => toggleLogExpand(),
  'net-detail': () => showNetDetail(),
  'autostart': () => toggleAutostart(),
  'update-download': () => downloadUpdate(),
  'upd-banner-close': () => dismissUpdateBanner(),
  // -- contrôles de fenêtre (desktop Tauri : sans effet dans un navigateur)
  'win-min': () => winAction('min'),
  'win-max': () => winAction('max'),
  'win-close': () => winAction('close'),
  // -- explorateur
  'sort': el => applySort(el.value),
  'srv-filter': () => renderServers(),
  'clear-filter': () => clearFilter(),
  'exp-state': el => setExpStateFilter(el.dataset.mode),
  'show-hidden': () => showHiddenServers(),
  'group-toggle': el => toggleGroup(el.dataset.key),
  'add-server': () => toggleAdd(),
  'select': (el, ev) => handleServerClick(el.dataset.id, ev),
  'row-act': el => act(el.dataset.op, el.dataset.id),
  'set-auto': el => setAuto(el.dataset.id, el.dataset.on === '1'),
  'set-fav': el => setFavorite(el.dataset.id, el.dataset.on === '1'),
  'hide-server': el => hideServer(el.dataset.id),
  'remove-srv': el => removeSrv(el.dataset.id),
  'remove-seed': el => removeSeedForever(el.dataset.id),
  'ssh-forget': el => sshForget(el.dataset.id),
  'import-peer': el => importPeer(el.dataset.id),
  'follow-ann': el => followAnnouncement(el.dataset.name, el.dataset.port),
  // -- accueil et détail serveur
  'go-home': () => goHome(),
  'tile': el => tileFilter(el.dataset.mode),
  'tile-tools': () => tileSortTools(),
  'detail-tab': el => { detailTab = el.dataset.tab; renderDetail(); if (typeof swarmOnDetailTab === 'function') swarmOnDetailTab(detailTab, detailCache); },
  'tool-search': el => setToolSearch(el),
  'tool-search-clear': () => clearToolSearch(),
  'tools-all-on': el => setAllTools(el.dataset.id, true),
  'tools-all-off': el => setAllTools(el.dataset.id, false),
  'tool-expand': (el, ev) => handleToolClick(el, ev),
  'tool-toggle': el => toggleTool(el.dataset.id, el.dataset.tool),
  'copy-tool-schema': el => copyToolSchema(el.dataset.tool),
  'copy-tool-rpc': el => copyToolRpc(el.dataset.tool),
  'copy-tool-res': el => copyToolResult(el.dataset.id, el.dataset.tool),
  'tool-args-input': el => saveToolArgsDraft(el),
  'tool-args-format': el => formatToolArgsDraft(el.dataset.id, el.dataset.tool),
  'tool-args-reset': el => resetToolArgsDraft(el.dataset.id, el.dataset.tool),
  'tool-run': el => runToolTest(el.dataset.id, el.dataset.tool),
  'server-pair': el => serverPair(el.dataset.id),
  'copy-self': el => copyText(el.textContent),
  'copy-text': el => copyDataText(el),
  'copy-conf': el => copyConf(el),
  'set-start-cmd': el => setStartCmd(el.dataset.id),
  'ctx-item': el => ctxItem(el),
  // -- boutique
  'store-cat': el => { storeCat = el.dataset.cat; renderStoreItems(); },
  'store-search': () => renderStoreItems(),
  'store-clear-search': () => clearStoreSearch(),
  'store-reset-filters': () => resetStoreFilters(),
  'store-sort': () => renderStoreItems(),
  'store-hide-inst': () => toggleHideInst(),
  'store-check-versions': el => checkStoreVersions(el),
  'open-store-update': (el, ev) => { if (ev) ev.stopPropagation(); openStoreToUpdate(el.dataset.storeId); },
  'update-store': el => updateStore(el.dataset.id, el),
  'toggle-env-eye': el => toggleEnvEye(el),
  'install-store': el => installStore(el.dataset.id, el),
  'uninstall-store': el => uninstallStore(el.dataset.id, el),
  // -- machines distantes (SSH)
  'ssh-discover': el => sshDiscover(el.dataset.id, el.dataset.user),
  'ssh-bootstrap': el => sshBootstrap(el.dataset.id, el.dataset.user),
  'ssh-save': () => sshSave(),
  'peers-scan': () => { doPeers().then(() => load()).then(() => renderMachines()); },
  // -- réglages
  'set-pane': el => setPane(el.dataset.pane),
  'theme': el => applyThemeAndSave(el.dataset.theme),
  'text-bright': el => applyTextBrightAndSave(el.value),
  'text-color': el => applyTextColorAndSave(el.dataset.color),
  'text-style-reset': () => resetTextStyle(),
  'test-notify': () => testNotify(),
  'check-update': () => checkUpdateNow(),
  'copy-sha256': () => copySha256(),
  'release-page': () => openReleasePage(),
  'settings-close': () => closeSettings(),
  'save-settings': () => saveSettings(),
  'reset-state': () => resetState(document.getElementById('sResetWhat').value),
  'reset-runtime': () => resetRuntime(),
  // -- écran Accès
  'access-close': () => closeAccessDlg(),
  'access-tab': el => setAccessTab(el.dataset.atab),
  'access-refresh': () => refreshAccess(true),
  'confirm-approve': el => confirmApprove(el.dataset.id, el.dataset.mode),
  'confirm-deny': el => confirmDeny(el.dataset.id),
  'grant-revoke': el => grantRevoke(el.dataset.id),
  'client-filter': el => setClientFilter(el.dataset.mode),
  'client-right': el => clientToggleRight(el.dataset.id, el.dataset.right),
  'client-rotate': el => clientRotateModal(el.dataset.id),
  'client-revoke': el => clientRevokeConfirm(el.dataset.id),
  'pair-approve': el => pairApprove(el.dataset.id),
  'pair-reject': el => pairReject(el.dataset.id),
  'pair-right': el => pairRightChange(el),
  'close-other-sessions': () => closeOtherSessions(),
  'decision-filter': el => setDecisionFilter(el),
  // -- formulaires et dialogs
  'add-submit': (_el, ev) => addServer(ev),
  'login-submit': (_el, ev) => loginSubmit(ev),
  'transport': () => transportChanged(),
  'import-mcp': () => importMcp(),
  'forget-code': () => forgetCode(),
  'confirm-dlg-ok': () => confirmDlgOk(),
  'confirm-dlg-cancel': () => confirmDlgCancel(),
  'close-dialog': el => { const d = el.closest('dialog'); if (d && d.open) { d.close(); restoreFocus(); } },
};

/* Entrée du menu contextuel : l'index vient de data-i, la fonction du tableau
   d'items du menu courant — aucune valeur reçue n'est exécutée. */
function ctxItem(el) {
  const it = ctxItems[+el.dataset.i];
  if (!it || it.dis) return;
  const opener = ctxOpener;
  ctxClose(false);
  it.fn();
  if (!document.querySelector('dialog[open]')
      && (!document.activeElement || document.activeElement === document.body)
      && isFocusableVisible(opener)) {
    opener.focus();
  }
}

/* Aiguillage unique : le contrôle le plus proche porteur de data-act gagne
   (un bouton dans une ligne n'arme pas l'action de la ligne), puis la table
   décide. Une clé inconnue ne fait rien — le test Rust interdit les clés
   orphelines dans les deux sens.
   Filtre de type : un <form data-act> n'agit qu'au SUBMIT (sinon un clic dans
   un champ déclencherait l'envoi) ; un <select>/<input data-act> n'agit qu'au
   change/input (sinon son clic vaudrait choix). */
const CHAMPS = ['INPUT', 'SELECT', 'TEXTAREA'];
const isNonTextChamp = el => el.tagName === 'SELECT'
  || (el.tagName === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio'));
function runAction(ev) {
  const t = ev.target;
  const el = t && t.closest ? t.closest('[data-act]') : null;
  if (!el) return;
  const tag = el.tagName;
  if (ev.type === 'click' && (tag === 'FORM' || CHAMPS.includes(tag))) return;
  if ((ev.type === 'change' || ev.type === 'input') && !CHAMPS.includes(tag)) return;
  if (ev.type === 'input' && isNonTextChamp(el)) return;
  const fn = ACT[el.dataset.act];
  if (!fn) return;
  fn(el, ev);
}

loadFrameworks();
loadStoreLogos();
updateLogoutBtn();
applyTextStyle(document.documentElement.dataset.textColor || 'default', document.documentElement.dataset.textBright || '100');
load();
armPolling();
// retour de la fenêtre (tray → visible) : reprise immédiate du rafraîchissement
document.addEventListener('visibilitychange', () => { if (!document.hidden) load(); });
