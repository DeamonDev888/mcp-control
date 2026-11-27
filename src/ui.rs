//! Page UI : coquille HTML — les styles et scripts vivent dans des ASSETS
//! servis (src/ui/app.css, app-boot.js, app.js) : fin du monolithe inline,
//! CSP serrée possible, diffs d'UI lisibles, plus de contrainte de raw string.
//! Service : GET / (coquille), /app.css, /app-boot.js, /app.js (lib.rs) —
//! query ?v=VERSION pour le cache immuable des assets.
pub const INDEX_HTML: &str = r#"<!doctype html>
<html lang="fr">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>MCP Control</title>
<link rel="icon" type="image/svg+xml" href="/favicon.svg?v=__MCC_VERSION__">
<link rel="icon" type="image/x-icon" href="/favicon.ico?v=__MCC_VERSION__">
<link rel="stylesheet" href="/app.css?v=__MCC_VERSION__">
<link rel="stylesheet" href="/swarm.css?v=__MCC_VERSION__">
<script src="/app-boot.js?v=__MCC_VERSION__"></script>
<script src="/app.js?v=__MCC_VERSION__" defer></script>
<script src="/swarm.js?v=__MCC_VERSION__" defer></script>
</head>
<body>
<a href='#content' class="skip-link">Aller au contenu principal</a>

<header id="titlebar" data-tauri-drag-region="deep">
  <button type="button" id="tbExplToggle" class="tb-icon-btn" data-act="toggle-explorer" data-tauri-drag-region="false" aria-label="Replier ou d&eacute;plier l'explorateur (Ctrl+B)" aria-expanded="true" aria-controls="explorer" title="Replier / d&eacute;plier l'explorateur (Ctrl+B)">
    <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x='3' y='4' width='18' height='16' rx='2'/><path d='M9 4v16'/></svg>
  </button>
  <button type="button" class="logo" data-act="go-home" id="tbHome" data-tauri-drag-region="false" aria-label="MCP Control — Retour à l'accueil (tableau de bord)" title="Retour à l'accueil — Tableau de bord MCP Control">
    <img class="mark" src="/favicon.svg?v=__MCC_VERSION__" alt="" aria-hidden="true">
    <span class="wordmark">MCP<span class="wm-dim"> Control</span></span>
  </button>
  <span class="tb-badge ok" title="Serveurs MCP qui répondent, sur le total suivi">UP <b id="stUp">0</b>/<b id="stTotal">0</b></span>
  <span class="tb-badge" title="Outils MCP exposés par les serveurs UP">outils <b id="stTools">0</b></span>
  <button type="button" id="tbLogout" class="tb-logout" hidden data-act="logout"
          aria-label="Oublier le code d'acc&egrave;s enregistr&eacute; dans ce navigateur"
          title="Oublier le code d'accès enregistré dans ce navigateur"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x='4.5' y='10.5' width='15' height='10' rx='2'/><path d='M8 10.5V7a4 4 0 0 1 8 0v3.5'/></svg></button>
  <span class="tb-spacer"></span>
  <div class="tb-actions" role="toolbar" aria-label="Actions principales">
    <button type="button" id="stUpdate" class="sb-upd" hidden data-act="update-download"
      aria-label="Mettre &agrave; jour l'application"
      title="T&eacute;l&eacute;charge directement la mise &agrave; jour pour cette machine"></button>
    <div id="tbDiscoverWrap" class="tb-menu-wrap">
      <button type="button" id="tbDiscover" class="primary tb-menu-btn" data-act="discover-toggle"
              aria-label="D&eacute;couverte — scanner les ports, le r&eacute;seau ou les machines"
              aria-haspopup="menu" aria-expanded="false" aria-controls="discoverMenu"
              title="D&eacute;couverte : scanner les ports locaux, le r&eacute;seau ou g&eacute;rer les machines distantes">
        <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='12' cy='12' r='8.5'/><circle cx='12' cy='12' r='3.5'/><path d='M12 3.5V7M12 17v3.5M3.5 12H7M17 12h3.5'/></svg>
        <span class="tb-lbl">D&eacute;couverte</span>
        <svg class="ic tb-caret" viewBox="0 0 24 24" aria-hidden="true"><path d='m6 9 6 6 6-6'/></svg>
      </button>
      <div id="discoverMenu" class="tb-menu" role="menu" aria-labelledby="tbDiscover" data-tauri-drag-region="false" hidden>
        <button type="button" class="tb-menu-item" role="menuitem" data-act="scan" title="Balayer les ports locaux pour d&eacute;couvrir les serveurs MCP">
          <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='11' cy='11' r='7'/><path d='m21 21-4.3-4.3'/></svg>
          <span class="tb-menu-txt"><span class="tb-menu-t">Scanner les ports</span><span class="tb-menu-sub">Ports locaux de cette machine</span></span>
        </button>
        <button type="button" class="tb-menu-item" role="menuitem" data-act="scan-net" title="Balayer le r&eacute;seau local pour d&eacute;couvrir les serveurs MCP distants">
          <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='12' cy='12' r='8.5'/><path d='M3.5 12h17M12 3.5c2.4 2.4 3.8 5.3 3.8 8.5s-1.4 6.1-3.8 8.5c-2.4-2.4-3.8-5.3-3.8-8.5s1.4-6.1 3.8-8.5Z'/></svg>
          <span class="tb-menu-txt"><span class="tb-menu-t">Scanner le r&eacute;seau</span><span class="tb-menu-sub">Serveurs MCP sur le LAN</span></span>
        </button>
        <button type="button" class="tb-menu-item" role="menuitem" data-act="machines" title="Machines distantes (SSH) et d&eacute;tection des moteurs MCP Control du LAN">
          <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x='2.5' y='4' width='19' height='13' rx='2'/><path d='M8 21h8M12 17v4'/></svg>
          <span class="tb-menu-txt"><span class="tb-menu-t">Machines</span><span class="tb-menu-sub">H&ocirc;tes SSH &amp; moteurs du LAN</span></span>
        </button>
      </div>
    </div>
    <button type="button" data-act="store" aria-label="Boutique de serveurs MCP" title="Catalogue de serveurs MCP pr&ecirc;ts &agrave; installer"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='9' cy='20' r='1.4'/><circle cx='17' cy='20' r='1.4'/><path d='M2.5 3.5h2.2l2.5 11.5h10.3l2-8H6'/></svg><span class="tb-lbl">Boutique</span></button>
    <button type="button" id="tbAccess" data-act="access-open" aria-label="Acc&egrave;s et s&eacute;curit&eacute;" title="Acc&egrave;s &mdash; clients appair&eacute;s, demandes d'appairage et confirmations avant ex&eacute;cution"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M12 3 4.5 6v5.5c0 4.4 3.1 8.3 7.5 9.5 4.4-1.2 7.5-5.1 7.5-9.5V6L12 3Z'/><path d='m8.8 11.8 2.2 2.2 4.2-4.4'/></svg><span class="tb-lbl">Acc&egrave;s</span><span id="tbAccessN" class="tb-count" hidden>0</span></button>
    <button type="button" data-act="settings-open" aria-label="R&eacute;glages du client" title="R&eacute;glages du client"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='12' cy='12' r='3.1'/><path d='M12 2.8v2.6M12 18.6v2.6M2.8 12h2.6M18.6 12h2.6M5.5 5.5l1.9 1.9M16.6 16.6l1.9 1.9M18.5 5.5l-1.9 1.9M7.4 16.6l-1.9 1.9'/></svg><span class="tb-lbl">R&eacute;glages</span></button>
    <button type="button" data-act="refresh-all" aria-label="Re-sonder tous les serveurs connus et d&eacute;couvrir les nouveaux" title="Re-sonder tous les serveurs connus et d&eacute;couvrir les nouveaux (tous ports en &eacute;coute)"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M21 12a9 9 0 1 1-2.6-6.4L21 8'/><path d='M21 3v5h-5'/></svg></button>
  </div>
  <div id="winCtrls">
    <button type="button" class="wbtn" id="wMin" data-act="win-min" aria-label="R&eacute;duire la fen&ecirc;tre" title="R&eacute;duire">&#x2500;</button>
    <button type="button" class="wbtn" id="wMax" data-act="win-max" aria-label="Agrandir ou restaurer la fen&ecirc;tre" title="Agrandir / restaurer">&#x25A1;</button>
    <button type="button" class="wbtn wclose" id="wClose" data-act="win-close" aria-label="Fermer la fen&ecirc;tre" title="Fermer">&#x2715;</button>
  </div>
</header>

<div id="deadBanner" hidden aria-live="polite" role="status"></div>

<div id="accessBanner" hidden aria-live="polite" role="status"><button type="button" id="accessBannerBtn" class="access-banner-btn" data-act="access-open" data-pending="1" title="Une demande attend une d&eacute;cision locale &mdash; cliquer pour ouvrir l'&eacute;cran Acc&egrave;s"></button></div>

<div id="updateBanner" hidden aria-live="polite" role="status">
  <div class="upd-b-main">
    <span class="upd-b-dot" aria-hidden="true"></span>
    <span id="updBannerTxt" class="upd-b-txt">Mise &agrave; jour en cours&hellip;</span>
    <span id="updBannerMeta" class="upd-b-meta"></span>
    <button type="button" id="updBannerClose" class="upd-b-close" data-act="upd-banner-close" hidden title="Fermer le bandeau">&times;</button>
  </div>
  <progress id="updBannerProg" class="upd-prog" max="100"></progress>
</div>

<div id="main">
  <aside id="explorer" aria-label="Explorateur de serveurs MCP">
    <div class="sec-label sec-label-tabs" title="Glisser-déposer une ligne pour r&eacute;ordonner (pr&eacute;f&eacute;rence d'affichage et priorit&eacute; de sondage) — ou reclasser via le menu : l'ordre appliqué est persisté">
      <div class="expl-tabs" role="tablist" aria-label="Vue de l'explorateur">
        <button type="button" id="explTabMcp" class="expl-tab sel" role="tab" aria-selected="true" data-act="expl-tab" data-tab="mcp">Serveurs MCP</button>
        <button type="button" id="explTabSwarm" class="expl-tab" role="tab" aria-selected="false" data-act="expl-tab" data-tab="swarm" title="Swarm Control — Essaim, agents MCP persistants &amp; A2A"><img class="ic-hornet" src="/swarm-logo/logo.svg?v=__MCC_VERSION__" alt="" aria-hidden="true"><span>Swarm</span><span id="tbSwarmN" class="tb-count" hidden>0</span></button>
      </div>
      <select id="sortSel" data-act="sort" aria-label="Trier les serveurs MCP" title="Reclasser les serveurs — « manuel » rend l'ordre pos&eacute; au glisser-déposer avant le tri">
        <option value="manuel">⇅ manuel</option>
        <option value="az">A→Z</option>
        <option value="etat">état</option>
        <option value="port">port</option>
        <option value="latence">latence</option>
        <option value="tools">tools</option>
        <option value="framework">framework</option>
        <option value="transport">transport</option>
      </select>
    </div>
    <div id="srvFilterWrap">
      <label for="srvFilter" class="sr-only">Filtrer les serveurs</label>
      <input id="srvFilter" placeholder="Filtrer les serveurs&hellip;"
        aria-label="Filtrer les serveurs par nom, port, framework ou &eacute;tat"
        title="Filtre par nom, port, framework ou &eacute;tat &mdash; raccourcis : / ou Ctrl+K, &Eacute;chap efface"
        data-act="srv-filter">
      <button type="button" id="srvFilterClear" hidden data-act="clear-filter" aria-label="Effacer le filtre" title="Effacer le filtre">&times;</button>
    </div>
    <div class="exp-pills" id="expStatePills" role="group" aria-label="Filtres rapides par &eacute;tat">
      <button type="button" class="exp-pill sel" data-act="exp-state" data-mode="all" aria-pressed="true">Tous</button>
      <button type="button" class="exp-pill" data-act="exp-state" data-mode="up" aria-pressed="false">UP</button>
      <button type="button" class="exp-pill" data-act="exp-state" data-mode="down" aria-pressed="false">Panne</button>
      <button type="button" class="exp-pill" data-act="exp-state" data-mode="locked" aria-pressed="false">Verrouill&eacute;s</button>
      <button type="button" class="exp-pill" data-act="exp-state" data-mode="paused" aria-pressed="false">Pause</button>
      <button type="button" class="exp-pill" data-act="exp-state" data-mode="swarm" aria-pressed="false">Swarm</button>
    </div>
    <button type="button" id="hiddenBar" hidden data-act="show-hidden"
      title="Les serveurs masqu&eacute;s ne le restent que pour cette session : ils reviennent &agrave; la r&eacute;ouverture de l'application (ou d'un rechargement de la page)"></button>
    <div id="localSec">
      <div class="grphead" id="localHead"><button type="button" class="grp-toggle" data-act="group-toggle" data-key="local" aria-expanded="true" aria-controls="srvList" title="Serveurs MCP locaux &agrave; cette machine (loopback) — cliquer pour replier/d&eacute;plier"><span class="gchev" aria-hidden="true">&#9662;</span><span class="gname">Serveurs locaux</span><span class="gcount" id="localCount"></span></button></div>
      <div class="grpbody" id="srvList" role="list" aria-label="Serveurs locaux"></div>
    </div>
    <div class="sec-label" id="remoteLabel" hidden>Machines distantes</div>
    <div id="remoteGroups"></div>
    <div class="sec-label">Machines &amp; pairs</div>
    <div id="peerList"></div>
    <div id="annSec" hidden>
      <div class="sec-label">D&eacute;tect&eacute;s, non suivis</div>
      <div id="annList"></div>
    </div>
    <div id="addWrap"><button type="button" data-act="add-server">+ Ajouter un serveur</button></div>
  </aside>
  <div id="grip" role="separator" aria-orientation="vertical" tabindex="0" aria-valuemin="180" aria-valuemax="520" aria-valuenow="320" aria-label="Ajuster la largeur de l'explorateur" title="Glisser ou utiliser les fl&egrave;ches pour ajuster la largeur"></div>
  <main id="content" tabindex="-1"><h1 class="sr-only">MCP Control</h1></main>
</div>

<div id="logpanel" role="region" aria-label="Journal d'&eacute;v&eacute;nements">
  <div id="loghead">
    <button type="button" id="logToggleBtn" class="log-toggle" data-act="log-toggle" aria-expanded="false" aria-controls="logbody" title="Afficher ou replier le journal"><span class="arrow" aria-hidden="true">&#9662;</span><span class="lbl">Journal</span><span id="logCountBadge" class="log-badge" hidden>0</span><span id="logErrBadge" class="log-badge err" hidden>0</span></button>
    <div id="logFilters" class="log-filters" role="group" aria-label="Filtrer le journal par niveau">
      <button type="button" class="log-fbtn sel" data-act="log-filter" data-level="all" aria-pressed="true">Tous</button>
      <button type="button" class="log-fbtn" data-act="log-filter" data-level="warn" aria-pressed="false">Alertes</button>
      <button type="button" class="log-fbtn" data-act="log-filter" data-level="error" aria-pressed="false">Erreurs</button>
    </div>
    <div class="log-tools">
      <button type="button" class="log-tbtn" data-act="log-copy" aria-label="Copier le journal visible" title="Copier le journal visible">Copier</button>
      <button type="button" class="log-tbtn" data-act="log-clear" aria-label="Effacer la vue du journal" title="Masquer les entr&eacute;es pr&eacute;c&eacute;dentes dans cette vue">Effacer</button>
      <button type="button" id="logExpandBtn" class="log-tbtn" data-act="log-expand" aria-pressed="false" aria-label="Agrandir ou r&eacute;duire la hauteur du journal" title="Agrandir / r&eacute;duire le journal">Hauteur</button>
    </div>
    <button type="button" id="netWarn" class="tb-badge netwarn" hidden data-act="net-detail"
      title="Les serveurs distants ne peuvent pas &ecirc;tre sond&eacute;s &mdash; VPN actif ? (Proton VPN : activer &laquo;&nbsp;autoriser les connexions LAN&nbsp;&raquo;)"></button>
    <button type="button" id="stAuto" class="sb-auto tb-push" role="switch" aria-checked="false" aria-label="Lancer au d&eacute;marrage de l'OS" data-act="autostart"
      title="Lancer l'application au d&eacute;marrage de l'OS (tray silencieux)"></button>
    <span class="tb-badge tb-hint">auto-d&eacute;tection : HTTP streamable &middot; stdio JSON-RPC</span>
    <span class="sb-ver" title="Version install&eacute;e">v__MCC_VERSION__</span>
  </div>
  <div id="logbody" aria-live="off"></div>
</div>

<dialog id="dlg" aria-labelledby="dlgAddTitle">
  <div class="dlg-head">
    <h2 id="dlgAddTitle"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M12 5v14M5 12h14'/></svg> Ajouter un serveur MCP</h2>
    <button type="button" class="close" data-act="close-dialog" aria-label="Fermer">&times;</button>
  </div>
  <div class="dlg-body">
    <form data-act="add-submit">
      <div class="row">
        <div><label for="fTransport">Transport</label>
          <select id="fTransport" data-act="transport">
            <option value="http">HTTP (streamable)</option>
            <option value="stdio">STDIO (process)</option>
          </select></div>
        <div><label for="fLabel">Nom affich&eacute;</label><input id="fLabel" placeholder="mon-serveur"></div>
      </div>
      <div id="httpFields">
        <div class="row">
          <div><label for="fPort">Port</label><input id="fPort" type="number" placeholder="3099"></div>
          <div><label for="fPath">Chemin MCP</label><input id="fPath" value="/mcp"></div>
        </div>
        <div class="row">
          <div><label for="fHttpHost">H&ocirc;te (optionnel)</label><input id="fHttpHost" placeholder="127.0.0.1 ou IP LAN/SSH"></div>
          <div class="chk-field"><input id="fViaSsh" type="checkbox"><label for="fViaSsh">Passer par SSH (machine enregistr&eacute;e)</label></div>
        </div>
        <div class="stack">
          <label for="fHttpProgram">Commande de lancement (option — permet Start/Stop par MCP Control)</label>
          <div class="row row-gap">
            <div><input id="fHttpProgram" placeholder="ex. node ou uvx mcp-server-x" aria-label="Programme de lancement HTTP"></div>
            <div><input id="fHttpWorkdir" placeholder="workdir (option)" aria-label="Dossier de travail HTTP"></div>
          </div>
          <input id="fHttpArgs" placeholder="args (ex. dist/server.js --port 3099)" aria-label="Arguments de lancement HTTP">
        </div>
      </div>
      <div id="stdioFields" hidden>
        <div class="row">
          <div><label for="fProgram">Programme</label><input id="fProgram" placeholder="node"></div>
          <div><label for="fWorkdir">Workdir (option)</label><input id="fWorkdir" placeholder="C:\chemin\serveur"></div>
        </div>
        <div class="stack"><label for="fArgs">Args (s&eacute;par&eacute;s par espace)</label><input id="fArgs" placeholder="dist/index.js --port 3000"></div>
      </div>
      <div class="row row-end">
        <button type="button" data-act="close-dialog">Annuler</button>
        <button type="submit" class="primary">Ajouter &amp; sonder</button>
      </div>
      <div class="dlg-import">
        <label for="fImport">Coller un bloc mcpServers ou une URL</label>
        <textarea id="fImport" rows="4" placeholder='{"mcpServers": {"nom": {"command": "npx", "args": ["-y", "paquet"]}}} &#8212; ou http://127.0.0.1:3099/mcp &#8212; ou claude mcp add nom npx paquet'></textarea>
        <div class="row row-end">
          <button type="button" class="primary" data-act="import-mcp">Importer</button>
        </div>
        <div id="importMsg" class="machine-msg" aria-live="polite"></div>
      </div>
    </form>
  </div>
</dialog>

<dialog id="storeDlg" aria-labelledby="dlgStoreTitle">
  <div class="dlg-head">
    <h2 id="dlgStoreTitle"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='9' cy='20' r='1.4'/><circle cx='17' cy='20' r='1.4'/><path d='M2.5 3.5h2.2l2.5 11.5h10.3l2-8H6'/></svg> Boutique &mdash; serveurs MCP <span id="storeCount" class="store-count"></span></h2>
    <button type="button" class="close" data-act="close-dialog" aria-label="Fermer">&times;</button>
  </div>
  <div class="dlg-body dlg-body-tight">
    <div class="store-head">
      <div class="store-bar">
        <div id="storeSearchWrap" class="store-search-wrap">
          <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='11' cy='11' r='7'/><path d='m21 21-4.3-4.3'/></svg>
          <label for="storeSearch" class="sr-only">Rechercher dans la boutique</label>
          <input id="storeSearch" placeholder="Rechercher un serveur MCP&hellip;" data-act="store-search">
          <button type="button" id="storeSearchClear" class="search-clear" hidden data-act="store-clear-search" aria-label="Effacer la recherche">&times;</button>
        </div>
        <label for="storeSort" class="sr-only">Trier les r&eacute;sultats</label>
        <select id="storeSort" data-act="store-sort" title="Trier les r&eacute;sultats">
          <option value="cat">Ordre du catalogue</option>
          <option value="az">A→Z</option>
        </select>
        <button type="button" id="storeHideInst" class="scat" role="switch" aria-checked="false" data-act="store-hide-inst"
          title="Masquer les serveurs d&eacute;j&agrave; pr&eacute;sents dans ta liste">Masquer les install&eacute;s</button>
      </div>
      <div id="storeCats" class="store-cats"></div>
      <div id="storeResults" class="store-results" aria-live="polite"></div>
    </div>
    <div id="storeGrid" class="store-grid"></div>
  </div>
</dialog>

<dialog id="machineDlg" aria-labelledby="dlgMachineTitle">
  <div class="dlg-head">
    <h2 id="dlgMachineTitle"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x='2.5' y='4' width='19' height='13' rx='2'/><path d='M8 21h8M12 17v4'/></svg> Machines distantes</h2>
    <button type="button" class="close" data-act="close-dialog" aria-label="Fermer">&times;</button>
  </div>
  <div class="dlg-body">
    <div class="conf-note">Connexion SSH par cl&eacute; (BatchMode &mdash; aucun mot de passe possible) : la machine
    doit accepter ta cl&eacute; publique. L'app inspecte l'OS et l'architecture distants, peut installer et
    d&eacute;marrer automatiquement le CLI officiel <b>mcp-control</b> (empreinte SHA-256 v&eacute;rifi&eacute;e),
    liste les ports en &eacute;coute, y sonde les serveurs MCP et permet de piloter leurs outils &agrave; distance.</div>
    <div class="machine-form">
      <label for="mIp" class="sr-only">IP ou h&ocirc;te SSH</label>
      <input id="mIp" placeholder="IP ou h&ocirc;te (ex. 192.168.1.42)">
      <label for="mUser" class="sr-only">Utilisateur SSH</label>
      <input id="mUser" placeholder="Utilisateur SSH">
      <label for="mPort" class="sr-only">Port SSH</label>
      <input id="mPort" type="number" min="1" max="65535" placeholder="Port (22)">
      <label for="mKey" class="sr-only">Cl&eacute; priv&eacute;e SSH (optionnel)</label>
      <input id="mKey" placeholder="Cl&eacute; SSH optionnelle (ex. ~/.ssh/id_ed25519)">
      <button type="button" class="primary" data-act="ssh-discover"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7'/><path d='M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7'/></svg> SSH &amp; d&eacute;couvrir</button>
      <button type="button" data-act="ssh-save"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M5 3.5h11L20.5 8v11a1.5 1.5 0 0 1-1.5 1.5H5A1.5 1.5 0 0 1 3.5 19V5A1.5 1.5 0 0 1 5 3.5Z'/><path d='M8 3.5V8h7V3.5M7.5 20.5v-6h9v6'/></svg> Enregistrer</button>
    </div>
    <div class="machine-opts">
      <input id="mAutoCli" type="checkbox" checked>
      <label for="mAutoCli">Auto-installer et d&eacute;marrer le CLI <b>mcp-control</b> si absent (v&eacute;rifi&eacute; SHA-256)</label>
    </div>
    <div id="machineMsg" class="machine-msg" aria-live="polite"></div>
    <div id="machineList"></div>
    <div class="stack"><button type="button" data-act="peers-scan"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M21 12a9 9 0 1 1-2.6-6.4L21 8'/><path d='M21 3v5h-5'/></svg> D&eacute;tecter les moteurs MCP Control sur le LAN</button></div>
  </div>
</dialog>

<dialog id="settingsDlg" class="settings-dlg" aria-labelledby="dlgSettingsTitle">
  <div class="dlg-head">
    <div class="dlg-title">
      <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="3.1" fill="none" stroke="currentColor" stroke-width="1.5"/>
        <path d="M12 2.6v3.2M12 18.2v3.2M2.6 12h3.2M18.2 12h3.2M5.3 5.3l2.3 2.3M16.4 16.4l2.3 2.3M18.7 5.3l-2.3 2.3M7.6 16.4l-2.3 2.3"
          fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"/>
      </svg>
      <div>
        <h2 id="dlgSettingsTitle">R&eacute;glages</h2>
        <p>Pr&eacute;f&eacute;rences du client MCP Control</p>
      </div>
    </div>
    <button type="button" class="close" data-act="settings-close" aria-label="Fermer" title="Fermer">&times;</button>
  </div>
  <div class="set-shell">
    <nav class="set-nav" id="setNav" role="tablist" aria-orientation="vertical" aria-label="Sections des r&eacute;glages">
      <button type="button" class="set-nav-btn sel" role="tab" id="stab-general" aria-selected="true" aria-controls="spane-general" tabindex="0" data-pane="general" data-act="set-pane"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='12' cy='12' r='3.1'/><path d='M12 2.8v2.6M12 18.6v2.6M2.8 12h2.6M18.6 12h2.6M5.5 5.5l1.9 1.9M16.6 16.6l1.9 1.9M18.5 5.5l-1.9 1.9M7.4 16.6l-1.9 1.9'/></svg>G&eacute;n&eacute;ral</button>
      <button type="button" class="set-nav-btn" role="tab" id="stab-decouverte" aria-selected="false" aria-controls="spane-decouverte" tabindex="-1" data-pane="decouverte" data-act="set-pane"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='11' cy='11' r='7'/><path d='m21 21-4.3-4.3'/></svg>D&eacute;couverte</button>
      <button type="button" class="set-nav-btn" role="tab" id="stab-interface" aria-selected="false" aria-controls="spane-interface" tabindex="-1" data-pane="interface" data-act="set-pane"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x='2.5' y='4' width='19' height='13' rx='2'/><path d='M8 21h8M12 17v4'/></svg>Interface</button>
      <button type="button" class="set-nav-btn" role="tab" id="stab-notifications" aria-selected="false" aria-controls="spane-notifications" tabindex="-1" data-pane="notifications" data-act="set-pane"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M6 9a6 6 0 0 1 12 0c0 5 2 6.5 2 6.5H4S6 14 6 9Z'/><path d='M10 19.5a2.2 2.2 0 0 0 4 0'/></svg>Notifications</button>
      <button type="button" class="set-nav-btn" role="tab" id="stab-updates" aria-selected="false" aria-controls="spane-updates" tabindex="-1" data-pane="updates" data-act="set-pane"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M12 15.5V4m0 0L7.5 8.5M12 4l4.5 4.5'/><path d='M4 16.5V19a1.5 1.5 0 0 0 1.5 1.5h13A1.5 1.5 0 0 0 20 19v-2.5'/></svg>Mises &agrave; jour</button>
      <div class="set-nav-sep" role="separator"></div>
      <button type="button" class="set-nav-btn danger" role="tab" id="stab-reset" aria-selected="false" aria-controls="spane-reset" tabindex="-1" data-pane="reset" data-act="set-pane"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M12 3.5 2.5 20h19L12 3.5Z'/><path d='M12 10v4.5M12 17.1v.1'/></svg>R&eacute;initialiser</button>
    </nav>
    <div class="set-panes">
      <section class="set-pane sel" role="tabpanel" id="spane-general" aria-labelledby="stab-general" data-pane="general">
        <div class="set-list">
          <div class="set-row">
            <div class="set-label"><label for="sRefresh">Intervalle de sondage</label>
              <em id="desc_refresh">D&eacute;lai entre deux cycles de sonde. Appliqu&eacute; au cycle suivant.</em>
              <div class="set-err" id="err_refresh" hidden></div></div>
            <div class="set-ctrl"><input id="sRefresh" type="number" min="5" max="3600" step="5" class="set-num" aria-describedby="desc_refresh err_refresh">
              <span class="set-unit">secondes</span></div>
          </div>
          <div class="set-row">
            <div class="set-label"><span>Th&egrave;me</span>
              <em>Apparence de l'interface. Conserv&eacute; sur ce poste.</em></div>
            <div class="set-ctrl seg" role="group" aria-label="Th&egrave;me">
              <button type="button" id="tDark" class="seg-btn" data-act="theme" data-theme="dark" aria-pressed="true" title="Th&egrave;me sombre (d&eacute;faut)">Sombre</button>
              <button type="button" id="tLight" class="seg-btn" data-act="theme" data-theme="light" aria-pressed="false" title="Th&egrave;me clair">Clair</button>
            </div>
          </div>
          <div class="set-row">
            <div class="set-label"><label for="sTextBright">Luminosit&eacute; des &eacute;critures</label>
              <em id="desc_text_bright">Ajuste l'&eacute;clat des textes dans toute l'interface (60&nbsp;% &agrave; 140&nbsp;%). Conserv&eacute; sur ce poste.</em></div>
            <div class="set-ctrl txt-bright-ctrl">
              <input id="sTextBright" type="range" min="60" max="140" step="5" value="100" class="txt-range" data-act="text-bright" aria-describedby="desc_text_bright">
              <span id="sTextBrightVal" class="set-static txt-bright-val">100&nbsp;%</span>
              <button type="button" id="btnTextReset" data-act="text-style-reset" title="R&eacute;tablir la luminosit&eacute; et la couleur par d&eacute;faut">D&eacute;faut</button>
            </div>
          </div>
          <div class="set-row col">
            <div class="set-label"><span>Couleur des &eacute;critures</span>
              <em>Teinte des textes (principaux et secondaires), calibr&eacute;e pour les th&egrave;mes sombre et clair.</em></div>
            <div class="set-ctrl txt-swatches" role="group" aria-label="Couleur des &eacute;critures">
              <button type="button" class="txt-sw-btn sel" data-act="text-color" data-color="default" aria-pressed="true"><span class="txt-sw-dot" aria-hidden="true"></span><span>D&eacute;faut</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="white" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Pur</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="warm" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Cr&egrave;me</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="amber" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Ambre</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="emerald" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Vert</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="cyan" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Cyan</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="blue" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Bleu</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="violet" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Violet</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="rose" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Rose</span></button>
              <button type="button" class="txt-sw-btn" data-act="text-color" data-color="lime" aria-pressed="false"><span class="txt-sw-dot" aria-hidden="true"></span><span>Matrix</span></button>
            </div>
            <div class="txt-preview" aria-hidden="true">
              <strong class="txt-prev-1">Texte principal</strong>
              <span class="txt-prev-2">Texte secondaire</span>
              <span class="txt-prev-3">Texte discret</span>
            </div>
          </div>
        </div>
      </section>

      <section class="set-pane" role="tabpanel" id="spane-decouverte" aria-labelledby="stab-decouverte" data-pane="decouverte">
        <div class="set-list">
          <div class="set-row">
            <div class="set-label"><label for="sScanFrom">Plage de scan de ports</label>
              <em id="desc_scan">Ports balay&eacute;s par &laquo; Scan ports &raquo;. La d&eacute;couverte native litemcp n'en d&eacute;pend pas.</em>
              <div class="set-err" id="err_scan" hidden></div></div>
            <div class="set-ctrl"><input id="sScanFrom" type="number" min="1" max="65535" class="set-num" aria-label="Port de d&eacute;but de scan" aria-describedby="desc_scan err_scan">
              <span class="set-dash">&ndash;</span>
              <input id="sScanTo" type="number" min="1" max="65535" class="set-num" aria-label="Port de fin de scan" aria-describedby="desc_scan err_scan"></div>
          </div>
          <div class="set-row">
            <div class="set-label"><span id="lbl_net_enabled">Pairs &amp; r&eacute;seau</span>
              <em id="desc_net">D&eacute;tecte les autres moteurs MCP Control du LAN et leurs serveurs distants.</em></div>
            <div class="set-ctrl">
              <label class="switch"><input type="checkbox" id="sNetEnabled" role="switch" aria-labelledby="lbl_net_enabled" aria-describedby="desc_net"><span class="switch-track"></span></label>
            </div>
          </div>
          <div class="set-row">
            <div class="set-label"><label for="sNetPorts">Ports UI des pairs</label>
              <em id="desc_ports">Ports interrog&eacute;s chez les pairs pour les rep&eacute;rer. S&eacute;par&eacute;s par des virgules.</em>
              <div class="set-err" id="err_ports" hidden></div></div>
            <div class="set-ctrl"><input id="sNetPorts" placeholder="5090, 5091" class="set-txt" aria-describedby="desc_ports err_ports"></div>
          </div>
          <div class="set-row">
            <div class="set-label"><label for="sDiscOnly">Ne sonder que ces ports (liste blanche)</label>
              <em id="desc_disc_only">Restreint la d&eacute;couverte locale aux seuls ports list&eacute;s. Vide = tous les ports en &eacute;coute.</em>
              <div class="set-err" id="err_disc_only" hidden></div></div>
            <div class="set-ctrl"><input id="sDiscOnly" placeholder="ex. 3000, 8787" class="set-txt" aria-describedby="desc_disc_only err_disc_only"></div>
          </div>
          <div class="set-row">
            <div class="set-label"><label for="sDiscExcl">Ports exclus de la d&eacute;couverte</label>
              <em id="desc_disc_excl">Ports locaux ignor&eacute;s lors de la d&eacute;couverte automatique des ports en &eacute;coute.</em>
              <div class="set-err" id="err_disc_excl" hidden></div></div>
            <div class="set-ctrl"><input id="sDiscExcl" placeholder="ex. 5432, 9222" class="set-txt" aria-describedby="desc_disc_excl err_disc_excl"></div>
          </div>
        </div>
      </section>

      <section class="set-pane" role="tabpanel" id="spane-interface" aria-labelledby="stab-interface" data-pane="interface">
        <div class="set-list">
          <div class="set-row">
            <div class="set-label"><label for="sUiBind">&Eacute;coute</label>
              <em id="desc_bind">127.0.0.1 = cette machine seule (session locale requise sur /api/*). 0.0.0.0 = accessible du LAN en TLS 1.3 obligatoire avec signatures Ed25519 RFC 9421 sur /mcp. Appliqu&eacute; au prochain d&eacute;marrage.</em></div>
            <div class="set-ctrl">
              <select id="sUiBind" aria-describedby="desc_bind">
                <option value="127.0.0.1">127.0.0.1</option>
                <option value="0.0.0.0">0.0.0.0</option>
              </select>
            </div>
          </div>
          <div class="set-row">
            <div class="set-label"><label for="sUiPort">Port</label>
              <em id="desc_uiport">Fig&eacute; au d&eacute;marrage du listener &mdash; modifiable dans config.json.</em></div>
            <div class="set-ctrl"><input id="sUiPort" type="number" disabled class="set-num" aria-describedby="desc_uiport"></div>
          </div>
        </div>
      </section>

      <section class="set-pane" role="tabpanel" id="spane-notifications" aria-labelledby="stab-notifications" data-pane="notifications">
        <div class="set-list">
          <div class="set-row">
            <div class="set-label"><span id="lbl_notif_desktop">Toast desktop</span>
              <em id="desc_notif_desktop">Notification Windows quand un serveur surveill&eacute; tombe (UP &rarr; DOWN ou PORT-ONLY).</em></div>
            <div class="set-ctrl">
              <label class="switch"><input type="checkbox" id="sNotifDesktop" role="switch" aria-labelledby="lbl_notif_desktop" aria-describedby="desc_notif_desktop"><span class="switch-track"></span></label>
            </div>
          </div>
          <div class="set-row col">
            <div class="set-label"><label for="sWebhookUrl">Webhook</label>
              <em id="desc_webhook">Requ&ecirc;te JSON &agrave; chaque incident (compatible Slack, Discord, n8n, endpoint maison).</em>
              <div class="set-err" id="err_webhook" hidden></div></div>
            <div class="set-ctrl set-ctrl-wide">
              <input id="sWebhookUrl" placeholder="http://hooks.exemple.com/services/&#8230;" aria-describedby="desc_webhook err_webhook">
            </div>
          </div>
          <div class="set-row">
            <div class="set-label"><label for="sWebhookMethod">M&eacute;thode HTTP</label>
              <em id="desc_wh_method">Verbe utilis&eacute; pour le webhook.</em></div>
            <div class="set-ctrl">
              <select id="sWebhookMethod" aria-describedby="desc_wh_method">
                <option value="POST">POST</option>
                <option value="PUT">PUT</option>
              </select>
            </div>
          </div>
          <div class="set-row col">
            <div class="set-label"><label for="sWebhookHeaders">En-t&ecirc;tes additionnels</label>
              <em id="desc_wh_headers">Une paire &laquo; Cl&eacute;: valeur &raquo; par ligne (ex. Authorization: Bearer&#8230;).</em></div>
            <div class="set-ctrl set-ctrl-wide">
              <textarea id="sWebhookHeaders" rows="3" placeholder="Authorization: Bearer &#8230;" aria-describedby="desc_wh_headers"></textarea>
            </div>
          </div>
          <div class="set-row">
            <div class="set-label"><span>&Eacute;tats notifi&eacute;s</span>
              <em id="desc_states">Les notifications se d&eacute;clenchent &agrave; la transition depuis UP.</em>
              <div class="set-err" id="err_states" hidden></div></div>
            <div class="set-ctrl set-inline">
              <label class="set-chk"><input type="checkbox" id="sStDown" aria-describedby="desc_states err_states"> DOWN</label>
              <label class="set-chk"><input type="checkbox" id="sStPortOnly" aria-describedby="desc_states err_states"> PORT-ONLY</label>
            </div>
          </div>
          <div class="set-row">
            <div class="set-label"><label for="sNotifMin">D&eacute;lai minimum</label>
              <em id="desc_min">Anti-spam : pas deux notifications du m&ecirc;me serveur dans cet intervalle.</em>
              <div class="set-err" id="err_min" hidden></div></div>
            <div class="set-ctrl"><input id="sNotifMin" type="number" min="0" max="86400" step="30" class="set-num" aria-describedby="desc_min err_min">
              <span class="set-unit">secondes</span></div>
          </div>
          <div class="set-row">
            <div class="set-label"><span>Test</span>
              <em>Enregistre la section puis &eacute;met une notification sur chaque canal activ&eacute;.</em></div>
            <div class="set-ctrl"><button type="button" data-act="test-notify"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M6 9a6 6 0 0 1 12 0c0 5 2 6.5 2 6.5H4S6 14 6 9Z'/><path d='M10 19.5a2.2 2.2 0 0 0 4 0'/></svg> Tester</button></div>
          </div>
          <div class="set-row" hidden id="notifyMsgRow">
            <div class="set-ctrl set-ctrl-wide"><span id="notifyMsg" class="set-note" aria-live="polite"></span></div>
          </div>
        </div>
      </section>

      <section class="set-pane" role="tabpanel" id="spane-updates" aria-labelledby="stab-updates" data-pane="updates">
        <div class="set-list">
          <div class="set-row">
            <div class="set-label"><span>Version install&eacute;e</span>
              <em>Release GitHub du repo DeamonDev888/mcp-control.</em></div>
            <div class="set-ctrl"><span class="set-static" id="sVersion">v&ndash;</span></div>
          </div>
          <div class="set-row">
            <div class="set-label"><span>Derni&egrave;re release</span>
              <em id="updMsg">&nbsp;</em></div>
            <div class="set-ctrl">
              <span class="set-static" id="sLatest">&ndash;</span>
              <button type="button" data-act="check-update" title="Interroge l'API GitHub releases/latest maintenant"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M21 12a9 9 0 1 1-2.6-6.4L21 8'/><path d='M21 3v5h-5'/></svg> V&eacute;rifier</button>
              <button type="button" class="primary" id="btnUpdInstall" data-act="update-download" title="T&eacute;l&eacute;charge, v&eacute;rifie (SHA-256), installe et relance automatiquement la nouvelle version"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M12 15.5V4m0 0L7.5 8.5M12 4l4.5 4.5'/><path d='M4 16.5V19a1.5 1.5 0 0 0 1.5 1.5h13A1.5 1.5 0 0 0 20 19v-2.5'/></svg> Mettre &agrave; jour &amp; relancer</button>
              <button type="button" data-act="release-page" title="Toutes les versions et archives"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M14 4h6v6M20 4l-9 9'/><path d='M19 14v5a1.5 1.5 0 0 1-1.5 1.5h-12A1.5 1.5 0 0 1 4 19V7.5A1.5 1.5 0 0 1 5.5 6H10'/></svg> Releases</button>
            </div>
          </div>
          <div class="set-row col" id="updProgRow" hidden>
            <div class="upd-set-head">
              <span id="updProgPhase" class="upd-set-phase">Mise &agrave; jour en cours&hellip;</span>
              <span id="updProgMeta" class="upd-set-meta"></span>
            </div>
            <progress id="updProgBar" class="upd-prog upd-prog-lg" max="100"></progress>
          </div>
          <div class="set-row col" id="sSha256Row" hidden>
            <div class="set-label"><span>Empreinte SHA-256 attendue (<span id="sSha256Asset">&ndash;</span>)</span>
              <em id="sSha256Note">Publi&eacute;e dans SHA256SUMS.txt de la release GitHub pour v&eacute;rifier l'int&eacute;grit&eacute; de l'installateur.</em></div>
            <div class="set-ctrl set-ctrl-wide">
              <code id="sSha256Val" class="mono-inline">&ndash;</code>
              <button type="button" id="sSha256Copy" data-act="copy-sha256" hidden title="Copier l'empreinte SHA-256">Copier SHA-256</button>
            </div>
          </div>
        </div>
      </section>

      <section class="set-pane" role="tabpanel" id="spane-reset" aria-labelledby="stab-reset" data-pane="reset">
        <div class="set-list">
          <div class="set-row">
            <div class="set-label"><label for="sResetWhat">Remettre &agrave; z&eacute;ro</label>
              <em id="desc_reset">Placement des serveurs, d&eacute;tections import&eacute;es et oublis. Irr&eacute;versible.</em></div>
            <div class="set-ctrl">
              <select id="sResetWhat" aria-describedby="desc_reset">
                <option value="placement">Placement des serveurs</option>
                <option value="detection">D&eacute;tections &amp; importations</option>
                <option value="oublis">Oublis (seeds + machines)</option>
                <option value="tout">Tout</option>
              </select>
              <button type="button" class="danger" data-act="reset-state">R&eacute;initialiser</button>
            </div>
          </div>
          <div class="set-row" id="resetMsgRow" hidden>
            <div class="set-ctrl set-ctrl-wide"><span id="resetMsg" class="set-note" aria-live="polite"></span></div>
          </div>
          <div class="set-row">
            <div class="set-label"><span>Historique runtime</span>
              <em>Efface crashs, relances et compteurs d'&eacute;chec enregistr&eacute;s par serveur. Ne touche ni la liste ni les r&eacute;glages.</em></div>
            <div class="set-ctrl"><button type="button" class="danger" data-act="reset-runtime">Effacer</button></div>
          </div>
          <div class="set-row" hidden id="runtimeMsgRow">
            <div class="set-ctrl set-ctrl-wide"><span id="runtimeMsg" class="set-note" aria-live="polite"></span></div>
          </div>
        </div>
      </section>
    </div>
  </div>
  <div id="settingsMsg" class="machine-msg set-msgline" aria-live="polite"></div>
  <div class="dlg-foot">
    <span id="settingsDirty" class="set-dirty" hidden>&#9679; Modifications non enregistr&eacute;es</span>
    <span class="dlg-foot-note" id="settingsFootNote"></span>
    <div class="row">
      <button type="button" data-act="settings-close">Annuler</button>
      <button type="button" class="primary" data-act="save-settings">Enregistrer</button>
    </div>
  </div>
</dialog>

<dialog id="accessDlg" class="settings-dlg access-dlg" aria-labelledby="dlgAccessTitle">
  <div class="dlg-head">
    <div class="dlg-title">
      <svg class="mark" viewBox="0 0 24 24" aria-hidden="true">
        <path d='M12 3 4.5 6v5.5c0 4.4 3.1 8.3 7.5 9.5 4.4-1.2 7.5-5.1 7.5-9.5V6L12 3Z'
          fill='none' stroke='currentColor' stroke-width='1.5' stroke-linejoin='round'/>
        <path d='m8.8 11.8 2.2 2.2 4.2-4.4' fill='none' stroke='currentColor' stroke-width='1.5'
          stroke-linecap='round' stroke-linejoin='round'/>
      </svg>
      <div>
        <h2 id="dlgAccessTitle">Acc&egrave;s &amp; S&eacute;curit&eacute;</h2>
        <p>Clients appair&eacute;s, demandes d'appairage, confirmations, identit&eacute; et journal d'audit</p>
      </div>
    </div>
    <button type="button" class="close" data-act="access-close" aria-label="Fermer" title="Fermer">&times;</button>
  </div>
  <div class="set-shell">
    <nav class="set-nav" id="accessNav" role="tablist" aria-orientation="vertical" aria-label="Sections de l'&eacute;cran Acc&egrave;s">
      <button type="button" class="set-nav-btn sel" role="tab" id="atab-confirmations" aria-selected="true" aria-controls="apane-confirmations" tabindex="0" data-atab="confirmations" data-act="access-tab"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M12 3 4.5 6v5.5c0 4.4 3.1 8.3 7.5 9.5 4.4-1.2 7.5-5.1 7.5-9.5V6L12 3Z'/><path d='m8.8 11.8 2.2 2.2 4.2-4.4'/></svg>Confirmations<span class="nav-count" id="accCountPending" hidden>0</span></button>
      <button type="button" class="set-nav-btn" role="tab" id="atab-appairage" aria-selected="false" aria-controls="apane-appairage" tabindex="-1" data-atab="appairage" data-act="access-tab"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M9.5 14.5 14.5 9.5'/><path d='M8 11 5.8 13.2a3.6 3.6 0 0 0 5 5L13 16'/><path d='M16 13l2.2-2.2a3.6 3.6 0 0 0-5-5L11 8'/></svg>Appairage<span class="nav-count" id="accCountPairs" hidden>0</span></button>
      <button type="button" class="set-nav-btn" role="tab" id="atab-clients" aria-selected="false" aria-controls="apane-clients" tabindex="-1" data-atab="clients" data-act="access-tab"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><circle cx='12' cy='8' r='3.4'/><path d='M5 20c0-3.6 3.1-6 7-6s7 2.4 7 6'/></svg>Clients</button>
      <button type="button" class="set-nav-btn" role="tab" id="atab-identite" aria-selected="false" aria-controls="apane-identite" tabindex="-1" data-atab="identite" data-act="access-tab"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x='4.5' y='10.5' width='15' height='10' rx='2'/><path d='M8 10.5V7a4 4 0 0 1 8 0v3.5'/></svg>Identit&eacute; &amp; Sessions</button>
      <button type="button" class="set-nav-btn" role="tab" id="atab-journal" aria-selected="false" aria-controls="apane-journal" tabindex="-1" data-atab="journal" data-act="access-tab"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M5 4.5h14v15H5z'/><path d='M8.5 8.5h7M8.5 12h7M8.5 15.5h4'/></svg>Journal d'audit</button>
    </nav>
    <div class="set-panes acc-panes">
      <section class="set-pane sel" role="tabpanel" id="apane-confirmations" aria-labelledby="atab-confirmations" data-atab="confirmations">
        <div id="accPending" class="acc-list"></div>
        <div id="accGrants" class="acc-list"></div>
      </section>
      <section class="set-pane" role="tabpanel" id="apane-appairage" aria-labelledby="atab-appairage" data-atab="appairage"><div id="accPairs" class="acc-list"></div></section>
      <section class="set-pane" role="tabpanel" id="apane-clients" aria-labelledby="atab-clients" data-atab="clients"><div id="accClients" class="acc-list"></div></section>
      <section class="set-pane" role="tabpanel" id="apane-identite" aria-labelledby="atab-identite" data-atab="identite"><div id="accIdentity" class="acc-list"></div></section>
      <section class="set-pane" role="tabpanel" id="apane-journal" aria-labelledby="atab-journal" data-atab="journal"><div id="accDecisions" class="acc-list"></div></section>
    </div>
  </div>
  <div id="accessMsg" class="machine-msg set-msgline" aria-live="polite"></div>
  <div class="dlg-foot">
    <span class="dlg-foot-note">Approbation r&eacute;serv&eacute;e &agrave; cette fen&ecirc;tre (session locale). Rien n'est persist&eacute; : un red&eacute;marrage redemande tout.</span>
    <div class="row"><button type="button" data-act="access-refresh">Rafra&icirc;chir</button></div>
  </div>
</dialog>

<dialog id="loginDlg" aria-labelledby="dlgLoginTitle">
  <div class="dlg-head">
    <h2 id="dlgLoginTitle"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><rect x='4.5' y='10.5' width='15' height='10' rx='2'/><path d='M8 10.5V7a4 4 0 0 1 8 0v3.5'/></svg> Acc&egrave;s prot&eacute;g&eacute;</h2>
  </div>
  <div class="dlg-body">
    <div class="conf-note"><b>Session locale expir&eacute;e ou acc&egrave;s distant</b> : en local, rouvre MCP Control depuis l'ic&ocirc;ne syst&egrave;me (tray) ou la console pour obtenir un nouveau jeton de session. En mode <b>api_access</b> h&eacute;rit&eacute; (lecture seule sur le LAN), saisis le code configur&eacute; dans <b>settings.api_access</b> :</div>
    <form data-act="login-submit">
      <label for="loginCode">Code d'acc&egrave;s (mode legacy api_access)</label>
      <input id="loginCode" type="password" autocomplete="off" placeholder="code d'acc&egrave;s">
      <div id="loginMsg" class="machine-msg" aria-live="polite"></div>
      <div class="row row-between">
        <button type="button" data-act="forget-code" title="Supprime le code enregistr&eacute; dans ce navigateur">Oublier le code enregistr&eacute;</button>
        <button type="submit" class="primary">D&eacute;verrouiller</button>
      </div>
    </form>
  </div>
</dialog>

<dialog id="confirmDlg" aria-labelledby="confirmDlgTitle" aria-describedby="confirmDlgMsg">
  <div class="dlg-head">
    <h2 id="confirmDlgTitle">Confirmation</h2>
    <button type="button" class="close" data-act="confirm-dlg-cancel" aria-label="Fermer">&times;</button>
  </div>
  <div class="dlg-body">
    <p id="confirmDlgMsg" class="confirm-msg"></p>
    <div id="confirmDlgDetail" class="confirm-detail" hidden></div>
    <div id="confirmDlgInputs" class="confirm-inputs" hidden></div>
    <div class="row row-end">
      <button type="button" id="confirmDlgCancel" data-act="confirm-dlg-cancel">Annuler</button>
      <button type="button" id="confirmDlgOk" class="primary" data-act="confirm-dlg-ok">Confirmer</button>
    </div>
  </div>
</dialog>

<div id="ctxmenu" class="ctxmenu" role="menu" hidden></div>
<div id="toast" popover="manual" aria-live="polite" role="status"></div>
</body>
</html>
"#;

/// Styles de l'UI — servis par GET /app.css (versionnés ?v=, cache immuable).
pub const APP_CSS: &str = include_str!("ui/app.css");

/// Amorçage thème AVANT le premier rendu (anti-flash) — bloquant dans <head>.
pub const APP_BOOT_JS: &str = include_str!("ui/app-boot.js");

/// Script principal de l'UI — servis par GET /app.js (defer, après le DOM).
pub const APP_JS: &str = include_str!("ui/app.js");

/// Styles dédiés à l'onglet Swarm Control — servis par GET /swarm.css.
pub const SWARM_CSS: &str = include_str!("ui/swarm.css");

/// Logique de l'onglet Swarm Control — servie par GET /swarm.js (defer).
pub const SWARM_JS: &str = include_str!("ui/swarm.js");

/// Logos SVG animés officiels de Swarm-control (`DeamonDev888/swarm-control`).
pub fn swarm_logo_svg(name: &str) -> Option<&'static str> {
    match name {
        "logo" => Some(include_str!("ui/swarm-logos/logo.svg")),
        "coordinator" => Some(include_str!("ui/swarm-logos/coordinator.svg")),
        "coder" => Some(include_str!("ui/swarm-logos/coder.svg")),
        "explorer" => Some(include_str!("ui/swarm-logos/explorer.svg")),
        "verifier" => Some(include_str!("ui/swarm-logos/verifier.svg")),
        "monitor" => Some(include_str!("ui/swarm-logos/monitor.svg")),
        "browser" => Some(include_str!("ui/swarm-logos/browser.svg")),
        "discord" => Some(include_str!("ui/swarm-logos/discord.svg")),
        "google" => Some(include_str!("ui/swarm-logos/google.svg")),
        "whatsapp" => Some(include_str!("ui/swarm-logos/whatsapp.svg")),
        "x" => Some(include_str!("ui/swarm-logos/x.svg")),
        "custom" => Some(include_str!("ui/swarm-logos/custom.svg")),
        _ => None,
    }
}

pub const FAVICON_SVG: &str = include_str!("ui/favicon.svg");

pub const FAVICON_ICO: &[u8] = include_bytes!("../desktop/icons/icon.ico");

#[cfg(test)]
mod tests {
    use super::{APP_BOOT_JS, APP_CSS, APP_JS, INDEX_HTML, SWARM_CSS, SWARM_JS};

    // ---------- contrat « tout bouton est câblé » --------------------------
    // Depuis le lot S2 il n'y a PLUS aucun attribut on* : chaque contrôle porte
    // data-act (clé de la table ACT de app.js) et la délégation centrale
    // l'aiguille. Le contrat a deux sens : aucune clé data-act orpheline dans
    // la page (bouton mort) et aucune action de la table jamais câblée
    // (code mort, ou faute de frappe dans un gabarit).

    /// Texte complet de l'UI : coquille HTML + script principal + module Swarm.
    fn ui_all() -> String {
        format!("{}\n{}\n{}", INDEX_HTML, APP_JS, SWARM_JS)
    }

    /// Attributs de gestionnaire en ligne : AUCUN n'est toléré (sonde).
    const ON_ATTRS: &[&str] = &[
        "onclick=",
        "onsubmit=",
        "onchange=",
        "oninput=",
        "ontoggle=",
        "ondragstart=",
        "ondragover=",
        "ondrop=",
        "ondragleave=",
        "ondragend=",
        "onerror=",
        "onload=",
    ];

    /// Mots-clés JS et fonctions natives : jamais des handlers applicatifs.
    const JS_NON_HANDLERS: &[&str] = &[
        "if",
        "else",
        "for",
        "while",
        "do",
        "switch",
        "case",
        "try",
        "catch",
        "finally",
        "return",
        "function",
        "typeof",
        "new",
        "delete",
        "void",
        "await",
        "async",
        "throw",
        "let",
        "const",
        "var",
        "setTimeout",
        "setInterval",
        "clearTimeout",
        "clearInterval",
        "alert",
        "confirm",
        "prompt",
        "fetch",
    ];

    /// Identificateurs suivis de « ( » dans le code d'un attribut on*.
    /// `obj.m(` (appel de méthode) et les mots-clés sont exclus — seuls les
    /// handlers globaux de la page comptent.
    fn calls_in(code: &str, out: &mut Vec<String>) {
        let b = code.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i].is_ascii_alphabetic() || b[i] == b'_' || b[i] == b'$' {
                let s = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'$')
                {
                    i += 1;
                }
                let name = &code[s..i];
                let mut j = i;
                while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                    j += 1;
                }
                let prev_dot = s > 0 && b[s - 1] == b'.';
                if j < b.len() && b[j] == b'(' && !prev_dot && !JS_NON_HANDLERS.contains(&name) {
                    out.push(name.to_string());
                }
            } else {
                i += 1;
            }
        }
    }

    /// Clés `data-act` présentes dans la page (coquille + gabarits régénérés
    /// par app.js) : chaque contrôle de l'UI passe par là.
    fn data_act_referenced() -> Vec<String> {
        let page = ui_all();
        let mut out = Vec::new();
        let mut rest: &str = &page;
        while let Some(i) = rest.find("data-act=\"") {
            let start = i + "data-act=\"".len();
            let end = rest[start..].find('"').map(|e| start + e).unwrap_or(start);
            let name = &rest[start..end];
            if !name.is_empty() {
                out.push(name.to_string());
            }
            rest = &rest[end..];
        }
        out.sort();
        out.dedup();
        out
    }

    /// Table action → fonction de app.js : lignes `'clé': code,` du bloc
    /// `const ACT = {` jusqu'à la première ligne `};`.
    fn act_table() -> Vec<(String, String)> {
        let src = APP_JS.replace("\r\n", "\n");
        let start = src
            .find("const ACT = {")
            .expect("table ACT absente de app.js");
        let rest = &src[start..];
        let end = rest.find("\n};").expect("fin de la table ACT introuvable");
        let mut out = Vec::new();
        for line in rest[..end].lines() {
            let line = line.trim();
            let Some(h) = line.strip_prefix('\'') else {
                continue;
            };
            let Some(q) = h.find('\'') else { continue };
            let Some(colon) = h[q..].find(':') else {
                continue;
            };
            let key = &h[..q];
            let code = h[q + colon + 1..].trim().trim_end_matches(',').trim();
            out.push((key.to_string(), code.to_string()));
        }
        out
    }

    fn defined_in_page(name: &str) -> bool {
        let page = ui_all();
        page.contains(&format!("function {name}("))
            || page.contains(&format!("const {name} ="))
            || page.contains(&format!("let {name} ="))
    }

    #[test]
    fn aucun_gestionnaire_en_ligne_dans_la_page() {
        let page = ui_all();
        let mut trouves = Vec::new();
        for attr in ON_ATTRS {
            let mut rest: &str = &page;
            while let Some(i) = rest.find(attr) {
                let line_start = rest[..i].rfind('\n').map(|n| n + 1).unwrap_or(0);
                let stop = (i + attr.len()).min(rest.len());
                trouves.push(format!("{} → {}", attr, rest[line_start..stop].trim()));
                rest = &rest[i + attr.len()..];
            }
        }
        assert!(
            trouves.is_empty(),
            "gestionnaires en ligne subsistants (refusés par script-src 'self') : {trouves:?}"
        );
        // style-src 'self' sans 'unsafe-inline' : aucun attribut style= ne doit
        // rester (les couleurs de frameworks sont passées en classes CSS).
        assert!(
            !page.contains("style=\""),
            "attribut style= subsistant : bloqué par la CSP style-src 'self'"
        );
        // aucun code construit à l'exécution depuis une valeur reçue
        for interdit in ["eval(", "new Function(", "javascript:"] {
            assert!(
                !APP_JS.contains(interdit) && !APP_BOOT_JS.contains(interdit),
                "construction de code interdite dans les scripts UI : {interdit}"
            );
        }
        assert!(
            !APP_BOOT_JS.contains("style=\""),
            "app-boot.js ne doit pas injecter de style= en ligne"
        );
        assert!(
            !APP_JS.contains(".style.") && !APP_BOOT_JS.contains(".style."),
            "aucune affectation JS .style.* ne doit subsister (CSP style-src 'self')"
        );
        assert!(
            !APP_JS.contains("const js ="),
            "le helper de construction de littéraux JS js() ne doit plus exister"
        );
    }

    #[test]
    fn chaque_data_act_est_dans_la_table_et_reciproquement() {
        let page = data_act_referenced();
        let table: Vec<String> = act_table().into_iter().map(|(k, _)| k).collect();
        assert!(
            page.len() >= 40,
            "l'inventaire data-act doit rester large (une refonte qui casse le \
             parseur doit être vue), trouvé : {page:?}"
        );
        let inconnues: Vec<&String> = page.iter().filter(|a| !table.contains(a)).collect();
        assert!(
            inconnues.is_empty(),
            "data-act sans entrée dans la table ACT (boutons morts) : {inconnues:?}"
        );
        let orphelines: Vec<&String> = table.iter().filter(|a| !page.contains(a)).collect();
        assert!(
            orphelines.is_empty(),
            "actions de la table câblées nulle part : {orphelines:?}"
        );
    }

    #[test]
    fn chaque_action_de_la_table_est_definie() {
        let mut morts = Vec::new();
        for (key, code) in act_table() {
            let mut names = Vec::new();
            calls_in(&code, &mut names);
            for n in names {
                if !defined_in_page(&n) {
                    morts.push(format!("{key} → {n}"));
                }
            }
        }
        assert!(
            morts.is_empty(),
            "actions branchées sur une fonction absente : {morts:?}"
        );
    }

    // ---------- contrat actions : l'UI n'appelle que des actions connues ----

    /// Actions d'API écrites en clair (`action: 'x'`) dans la page.
    fn actions_referenced() -> Vec<String> {
        let page = ui_all();
        let mut out = Vec::new();
        for needle in ["action: '", "'action': '"] {
            let mut rest: &str = &page;
            while let Some(i) = rest.find(needle) {
                let start = i + needle.len();
                let end = rest[start..]
                    .find(['\'', '"'])
                    .map(|e| start + e)
                    .unwrap_or(start);
                let name = &rest[start..end];
                if !name.is_empty() && name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                    out.push(name.to_string());
                }
                rest = &rest[end..];
            }
        }
        out.sort();
        out.dedup();
        out
    }

    #[test]
    fn chaque_action_api_de_l_ui_est_conue_par_le_moteur() {
        let actions = actions_referenced();
        assert!(
            actions.len() >= 20,
            "l'inventaire doit rester consistant, trouvé : {actions:?}"
        );
        let inconnues: Vec<&String> = actions
            .iter()
            .filter(|a| {
                !crate::manage::ACTIONS.contains(&a.as_str())
                    && !crate::auth::action_reservee_admin(a)
            })
            .collect();
        assert!(
            inconnues.is_empty(),
            "actions UI inconnues du dispatcher : {inconnues:?}"
        );
        // Les actions construites par variable — act(`${up?'kill':'start'}`) —
        // ne sont pas capturées ci-dessus : les littéraux de base sont couverts
        // ici, et chaque action de la surface l'est par la suite E2E.
        for a in ["start", "kill", "refresh", "toggle", "remove"] {
            assert!(
                crate::manage::ACTIONS.contains(&a),
                "action de base manquante au dispatcher : {a}"
            );
        }
    }

    #[test]
    fn les_valeurs_de_reset_state_sont_conues_par_le_moteur() {
        // les <option value> du select de réinitialisation pilotent reset_state
        for v in ["placement", "detection", "oublis", "tout"] {
            assert!(
                INDEX_HTML.contains(&format!("<option value=\"{v}\"")),
                "option reset_state manquante : {v}"
            );
        }
    }

    // ---------- contrat « assets servis » ----------------------------------

    #[test]
    fn la_coquille_reference_les_assets_versionnes() {
        for (url, cite) in [
            ("/app.css", APP_CSS),
            ("/app-boot.js", APP_BOOT_JS),
            ("/app.js", APP_JS),
        ] {
            assert!(
                INDEX_HTML.contains(&format!("{}?v=__MCC_VERSION__", url)),
                "asset non référencé (ou non versionné) dans la coquille : {url}"
            );
            assert!(!cite.trim().is_empty(), "asset vide : {url}");
        }
        assert!(
            APP_CSS.len() > 10_000 && APP_JS.len() > 50_000,
            "assets principaux anormalement petits : css={}, js={}",
            APP_CSS.len(),
            APP_JS.len()
        );
        // le placeholder de version est bien substitué côté serveur (lib.rs)
        assert!(APP_JS.contains("const MCC_VERSION = '__MCC_VERSION__'"));
        // l'amorçage thème doit rester MINUSCULE : il est chargé bloquant
        // dans <head>, chaque octet retarde le premier rendu
        assert!(
            APP_BOOT_JS.len() < 400,
            "app-boot.js gonfle le chemin critique : {} octets",
            APP_BOOT_JS.len()
        );
    }

    // ---------- Lot 4 GUI (audit 04, constats 2-8, 10, 11) -----------------
    // Contrats de présence : chaque correctif GUI a son marqueur dans la
    // coquille, le script ou la feuille de style. Le comportement réel
    // (arrêt/relance du moteur) est vérifié hors CI, moteur relancé sur un
    // port privé.

    /// Corps d'une fonction JS de la page : de sa signature à la première
    /// accolade fermante de colonne 0 (convention du fichier : aucun bloc
    /// interne ne ferme en colonne 0). Fins de ligne normalisées en LF.
    fn fn_body(src: &str, name: &str) -> String {
        let src = src.replace("\r\n", "\n");
        let start = src
            .find(&format!("function {name}("))
            .unwrap_or_else(|| panic!("fonction JS absente : {name}"));
        let rest = &src[start..];
        let end = rest.find("\n}").unwrap_or(rest.len());
        rest[..end].to_string()
    }

    #[test]
    fn lot4_bandeau_moteur_injoignable_est_cable() {
        // #2/#3 : bandeau dans la coquille, échecs de poll comptés, bandeau
        // armé dès le premier échec quand rien n'a jamais chargé (boot), et
        // après 2 échecs consécutifs quand l'état affiché est périmé ; il est
        // levé dès la première réponse. Le grisage des compteurs a son CSS.
        assert!(
            INDEX_HTML.contains(r#"id="deadBanner" hidden"#),
            "bandeau « moteur injoignable » absent de la coquille"
        );
        assert!(
            APP_JS.contains("moteur injoignable, nouvel essai"),
            "libellé boot « moteur injoignable, nouvel essai… » absent"
        );
        let load = fn_body(APP_JS, "load");
        assert!(
            load.contains("pollFails"),
            "load() doit compter les échecs consécutifs"
        );
        assert!(
            load.contains("setDead(true"),
            "load() doit armer le bandeau à l'échec"
        );
        assert!(
            load.contains("setDead(false"),
            "load() doit lever le bandeau dès que le moteur répond"
        );
        assert!(
            APP_JS.contains("pollFails >= 2"),
            "le bandeau ne doit apparaître qu'après 2 échecs consécutifs"
        );
        let dead = fn_body(APP_JS, "setDead");
        assert!(
            dead.contains("classList.toggle('dead'"),
            "setDead doit marquer body.dead (compteurs grisés)"
        );
        assert!(APP_CSS.contains("#deadBanner"), "CSS du bandeau absent");
        assert!(
            APP_CSS.contains("body.dead"),
            "règle de grisage des compteurs absente"
        );
        // #2 : l'intervalle est armé au boot, indépendamment du premier load()
        let js = APP_JS.replace("\r\n", "\n");
        assert!(
            js.contains("load();\narmPolling();"),
            "le polling doit être armé au démarrage, pas seulement après un render()"
        );
    }

    #[test]
    fn lot4_import_mcp_a_un_point_d_entree() {
        // #4 : le dialog « Ajouter » expose la zone de texte d'import et
        // envoie {action:"import_mcp", text} ; le handler est défini (contrat
        // « tout bouton est câblé » ci-dessus) et l'action est connue du
        // dispatcher (contrat actions ci-dessus).
        assert!(
            INDEX_HTML.contains(r#"id="fImport""#),
            "zone de texte d'import absente du dialog « Ajouter un serveur »"
        );
        assert!(
            INDEX_HTML.contains("Coller un bloc mcpServers"),
            "libellé de la zone d'import absent"
        );
        let h = fn_body(APP_JS, "importMcp");
        assert!(
            h.contains("action: 'import_mcp'"),
            "l'UI doit envoyer l'action import_mcp"
        );
        assert!(
            h.contains("text"),
            "le texte collé doit partir dans req.text"
        );
        assert!(
            h.contains("added"),
            "le nombre ajouté doit être affiché en résultat"
        );
        assert!(
            h.contains("skipped"),
            "les entrées écartées (avec leur motif) doivent être affichées si la réponse les porte"
        );
    }

    #[test]
    fn lot4_urls_ipv6_entourees_de_crochets() {
        // #5 : un seul helper, utilisé par les DEUX constructeurs d'URL
        // (« Conf agent » srvUrl et « Copier l'URL »/texte d'info ctxUrl).
        let uses = APP_JS.matches("hostForUrl").count();
        assert!(
            uses >= 3,
            "hostForUrl doit être défini et appelé par srvUrl et ctxUrl (déf + 2 appels), trouvé : {uses}"
        );
        assert!(
            !APP_JS.contains("'http://' + h + ':'"),
            "srvUrl construit encore une URL sans crochets IPv6"
        );
        assert!(
            !APP_JS.contains("'http://' + (ti.host || '127.0.0.1')"),
            "ctxUrl construit encore une URL sans crochets IPv6"
        );
    }

    #[test]
    fn lot4_hosts_loopback_classes_locaux() {
        // #6 : 127.0.0.1 / ::1 / localhost = CETTE machine — plus de groupe
        // « Machines distantes » ni de formulaire de commande supprimé.
        let uses = APP_JS.matches("isLoopbackHost").count();
        assert!(
            uses >= 3,
            "isLoopbackHost doit servir hostOf ET startCmdForm (déf + 2 appels), trouvé : {uses}"
        );
        for h in ["'127.0.0.1'", "'::1'", "'localhost'"] {
            assert!(
                APP_JS.contains(h),
                "hôte loopback manquant dans la liste : {h}"
            );
        }
        let f = fn_body(APP_JS, "startCmdForm");
        assert!(
            f.contains("isLoopbackHost"),
            "le formulaire de commande de lancement doit revenir pour un hôte loopback"
        );
    }

    #[test]
    fn lot4_refus_moteur_toaste_sur_setfavorite_setauto_scan() {
        // #7/#10 : un refus du moteur (j.ok=false) ne peut plus être
        // silencieux. Depuis le lot C (TODO-RESTES), la lecture de j.ok et le
        // toast de j.error vivent dans le helper unique postAction — setAuto
        // et setFavorite y passent ; doScan garde son toast préfixé (quiet)
        // plus le champ found et le bouton occupé.
        for name in ["setAuto", "setFavorite"] {
            let f = fn_body(APP_JS, name);
            assert!(
                f.contains("postAction("),
                "{name} doit passer par le helper postAction (refus moteur silencieux sinon)"
            );
        }
        let helper = fn_body(APP_JS, "postAction");
        assert!(
            helper.contains("j.ok") && helper.contains("toast("),
            "le helper postAction doit lire j.ok et toast le refus"
        );
        let scan = fn_body(APP_JS, "doScan");
        assert!(
            scan.contains("found"),
            "le scan doit toast le nombre trouvé (champ found s'il existe)"
        );
        assert!(
            scan.contains("disabled = true"),
            "le bouton de scan doit être occupé pendant le scan"
        );
    }

    #[test]
    fn lot4_barre_de_titre_repli_et_controles_de_fenetre() {
        // #8 : sous ~1120 px les libellés s'effacent (icônes seules +
        // infobulle) ; #winCtrls ne se comprime jamais — visible à toute
        // largeur, y compris la fenêtre desktop minimale (940 px).
        assert!(
            APP_CSS.contains(".tb-lbl"),
            "CSS de repli des libellés de la barre de titre absent"
        );
        assert!(
            APP_CSS.contains("@media (max-width:1120px)"),
            "breakpoint de repli (libellés masqués) absent"
        );
        assert!(
            APP_CSS.contains(".logo,.tb-actions,#winCtrls{flex-shrink:0}"),
            "#winCtrls doit être non compressible (visible à toute largeur)"
        );
        for lbl in [
            "D&eacute;couverte",
            "Boutique",
            "Acc&egrave;s",
            "R&eacute;glages",
        ] {
            assert!(
                INDEX_HTML.contains(&format!(r#"<span class="tb-lbl">{lbl}</span>"#)),
                "libellé de la barre de titre non encapsulé pour le repli : {lbl}"
            );
        }
        for item in ["Scanner les ports", "Scanner le r&eacute;seau", "Machines"] {
            assert!(
                INDEX_HTML.contains(&format!(r#"<span class="tb-menu-t">{item}</span>"#)),
                "entrée manquante dans le menu Découverte : {item}"
            );
        }
    }

    #[test]
    fn lot4_copy_conf_reuse_copy_text() {
        // #11 : le bouton « copier » de « Conf agent » passe par copyText —
        // le repli execCommand (copyFallback) couvre un refus du presse-papiers.
        let f = fn_body(APP_JS, "copyConf");
        assert!(
            f.contains("copyText("),
            "copyConf doit réutiliser copyText (repli copyFallback)"
        );
    }

    // ---------- Lot C GUI (TODO-RESTES, 3e passe) ---------------------------
    // #1 : un seul helper d'envoi d'action (postAction) — tous les fetch vers
    // /api/action y passent, il lit ok et toast error ; #2 : réessai rapide du
    // poll en panne moteur (2 s) ; #3 : bandeau immédiat sur erreur réseau
    // d'une action. Le comportement réel (délais, toasts, console) est mesuré
    // hors CI dans un navigateur sur une instance privée.

    #[test]
    fn restes_c_un_seul_helper_envoi_action_lit_ok_et_toaste() {
        // #1 : plus aucun fetch('/api/action') hors du helper ; il lit j.ok,
        // toast j.error en erreur et renvoie la réponse. Les dix appels
        // autrefois silencieux y passent tous.
        let n = APP_JS.matches("fetch('/api/action'").count();
        assert_eq!(
            n, 1,
            "fetch('/api/action') doit n'exister que dans postAction, trouvé : {n}"
        );
        let h = fn_body(APP_JS, "postAction");
        assert!(
            h.contains("fetch('/api/action'"),
            "le fetch restant doit être celui du helper postAction"
        );
        assert!(h.contains("j.ok"), "le helper doit lire j.ok");
        assert!(
            h.contains("toast("),
            "le helper doit afficher j.error en toast d'erreur"
        );
        assert!(h.contains("return j"), "le helper doit renvoyer la réponse");
        for name in [
            "act",
            "toggleAutostart",
            "postReorder",
            "removeSeedForever",
            "doScanNet",
            "doPeers",
            "importPeer",
            "sshForget",
            "toggleTool",
            "refreshAll",
        ] {
            let f = fn_body(APP_JS, name);
            assert!(
                f.contains("postAction("),
                "{name} doit passer par le helper postAction (refus silencieux sinon)"
            );
        }
        // refreshAll : sans toast par serveur (quiet), mais échec GLOBAL signalé.
        let ra = fn_body(APP_JS, "refreshAll");
        assert!(
            ra.contains("{quiet: true}"),
            "refreshAll ne doit pas toaster par serveur (option quiet)"
        );
        assert!(
            ra.contains("toast("),
            "refreshAll doit signaler un échec global"
        );
    }

    #[test]
    fn restes_c_reessai_rapide_moteur_mort() {
        // #2 : après le premier échec de poll, réessai à 2 s (POLL_RETRY_MS)
        // au lieu du prochain cycle complet — bandeau toujours au 2ᵉ échec,
        // mais ~refresh_secs+2 s au lieu de ~2×refresh_secs ; pendant la
        // panne chaque échec réarme le réessai rapide ; le rythme normal
        // (armPolling) revient dès que le moteur répond (render).
        let load = fn_body(APP_JS, "load");
        assert!(
            load.contains("POLL_RETRY_MS"),
            "load() doit réarmer le poll en réessai rapide après un échec"
        );
        assert!(
            APP_JS.contains("POLL_RETRY_MS = 2000"),
            "constante de réessai 2 s absente"
        );
        let arm = fn_body(APP_JS, "armPolling");
        assert!(
            arm.contains("refresh_secs"),
            "le rythme normal doit rester réglé par refresh_secs"
        );
        assert!(
            !APP_JS.contains("setInterval(load"),
            "le polling ne doit plus être un intervalle fixe (réessai rapide impossible sinon)"
        );
        assert!(
            APP_JS.contains("pollFails >= 2"),
            "le bandeau doit rester armé au 2ᵉ échec consécutif"
        );
    }

    #[test]
    fn restes_c_erreur_reseau_action_bandeau_immediat() {
        // #3 : une action qui ne part pas (erreur réseau, pas un refus) arme
        // le bandeau tout de suite et passe le poll en réessai rapide.
        let nd = fn_body(APP_JS, "netDown");
        assert!(
            nd.contains("setDead(true"),
            "netDown doit armer le bandeau immédiatement"
        );
        assert!(
            nd.contains("POLL_RETRY_MS"),
            "netDown doit passer le poll en réessai rapide"
        );
        let h = fn_body(APP_JS, "postAction");
        assert!(
            h.contains("netDown()"),
            "postAction doit signaler l'erreur réseau par netDown()"
        );
    }

    // ---------- Lot M3 : écran « Accès » ------------------------------------

    /// Le bloc JS de l'écran « Accès » — de son en-tête au bloc de polling.
    fn bloc_acces() -> &'static str {
        let debut = APP_JS
            .find("Écran « Accès »")
            .expect("en-tête du bloc Accès absent de app.js");
        let fin = APP_JS[debut..]
            .find("Polling arm")
            .map(|i| debut + i)
            .expect("fin du bloc Accès introuvable");
        &APP_JS[debut..fin]
    }

    /// Référence Rust de `esc()` : mêmes caractères dangereux, mêmes entités,
    /// mêmes caractères de contrôle retirés. Ce n'est pas une réécriture de la
    /// page : c'est la règle que la page doit appliquer, figée ici pour qu'un
    /// libellé hostile ne puisse pas passer.
    fn esc_html(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for c in s.chars() {
            if c.is_control() {
                continue;
            }
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&#39;"),
                autre => out.push(autre),
            }
        }
        out
    }

    /// **Le test exigé par le lot M3** : un libellé hostile (balise, guillemets,
    /// très long, caractères de contrôle) ne peut jamais atterrir dans le HTML.
    #[test]
    fn libelle_hostile_jamais_dans_le_html() {
        // 1) la règle d'échappement de la page est bien celle vérifiée ici
        let esc = APP_JS
            .lines()
            .find(|l| l.starts_with("const esc = "))
            .expect("définition de esc() absente");
        assert!(esc.contains(r#"[&<>"']/g"#), "caractères échappés : {esc}");
        assert!(
            esc.contains(r"[\u0000-\u001f\u007f]"),
            "les caractères de contrôle doivent être retirés par esc() : {esc}"
        );
        for entite in ["&amp;", "&lt;", "&gt;", "&quot;", "&#39;"] {
            assert!(esc.contains(entite), "entité manquante : {entite}");
        }
        // 2) les quatre familles hostiles du critère
        let tres_long = "A".repeat(4096);
        let hostiles: [&str; 5] = [
            "<script>alert('x')</script>",
            "\"><img src=x onerror=alert(1)>",
            "l'équipe \"rouge\" & <b>gras</b>",
            tres_long.as_str(),
            "ligne\u{0}nulle\u{1b}[31m\u{7f}cloche",
        ];
        for brut in hostiles {
            let rendu = esc_html(brut);
            assert!(!rendu.contains('<') && !rendu.contains('>'), "{rendu}");
            assert!(!rendu.contains('"'), "guillemet non échappé : {rendu}");
            assert!(!rendu.contains('\''), "apostrophe non échappée : {rendu}");
            assert!(
                !rendu.chars().any(|c| c.is_control()),
                "caractère de contrôle dans le rendu : {rendu:?}"
            );
            // rien n'est perdu hors les caractères dangereux (esc() ALLONGE le
            // texte : une entité vaut plusieurs caractères)
            let attendus = brut.chars().filter(|c| !c.is_control()).count();
            assert!(rendu.chars().count() >= attendus, "texte tronqué : {rendu}");
        }
        assert_eq!(
            esc_html(&"A".repeat(4096)).matches('A').count(),
            4096,
            "un libellé très long est conservé (c'est la page qui tronque à l'affichage)"
        );
        // le cas exact du critère : <script> ne survit jamais tel quel
        assert!(!esc_html("<script>").contains("script>"));
    }

    /// Statique : dans l'écran « Accès », **aucune** interpolation de gabarit
    /// n'insère un texte venu d'un client sans passer par `esc()` / `js()`.
    #[test]
    fn ecran_acces_echappe_tout_champ_venu_d_un_client() {
        let bloc = bloc_acces();
        // fragments locaux construits avec esc()/js() — vérifiés juste après
        let permis = [
            "esc(",
            "js(",
            "ic(",
            "effectHtml(",
            "boutons",
            "btnRevoque",
            "badgeRevoque",
            "cases",
            "classeOn",
            "classeOff",
        ];
        let mut i = 0usize;
        let mut total = 0usize;
        while let Some(k) = bloc[i..].find("${").map(|p| i + p) {
            let mut prof = 1usize;
            let mut m = k + 2;
            let mut quote: Option<char> = None;
            let octets = bloc.as_bytes();
            while m < bloc.len() && prof > 0 {
                let c = bloc[m..].chars().next().unwrap();
                match quote {
                    Some(q) => {
                        if c == '\\' {
                            m += 2;
                            continue;
                        }
                        if c == q {
                            quote = None;
                        }
                    }
                    None => match c {
                        '\'' | '"' | '`' => quote = Some(c),
                        '{' => prof += 1,
                        '}' => prof -= 1,
                        _ => {}
                    },
                }
                m += c.len_utf8();
            }
            assert!(
                prof == 0,
                "interpolation non fermée près de : {}",
                &bloc[k..]
            );
            let expr = bloc[k + 2..m - 1].trim();
            assert!(
                permis.iter().any(|p| expr.starts_with(p)),
                "texte non échappé dans l'écran Accès : ${{{expr}}}"
            );
            let _ = octets;
            total += 1;
            i = m;
        }
        assert!(
            total >= 30,
            "l'inventaire des interpolations doit rester consistant (trouvé {total})"
        );
        // les fragments autorisés sont eux-mêmes construits avec esc()/js()
        for (nom, extrait) in [
            ("boutons", "const boutons ="),
            ("cases", "const cases ="),
            ("btnRevoque", "const btnRevoque ="),
            ("badgeRevoque", "const badgeRevoque ="),
            ("classeOn", "const classeOn ="),
        ] {
            let pos = bloc
                .find(extrait)
                .unwrap_or_else(|| panic!("fragment {nom} introuvable"));
            let fin = (pos + 700).min(bloc.len());
            let corps = &bloc[pos..fin];
            assert!(
                corps.contains("esc(") || corps.contains("js("),
                "le fragment {nom} doit échapper ce qu'il contient : {corps}"
            );
        }
        // et aucun champ de client n'est interpolé nu
        for champ in [
            "label",
            "keyid",
            "fingerprint",
            "fingerprint_court",
            "address",
            "code",
            "tool",
            "paired_from",
            "created_at",
            "last_used_at",
            "verdict",
            "reason",
            "resume",
            "note",
        ] {
            for motif in [
                format!("${{{champ}}}"),
                format!("${{p.{champ}}}"),
                format!("${{c.{champ}}}"),
                format!("${{g.{champ}}}"),
                format!("${{x.{champ}}}"),
            ] {
                assert!(
                    !bloc.contains(&motif),
                    "champ venu d'un client interpolé sans esc() : {motif}"
                );
            }
        }
    }

    /// L'écran « Accès » parle aux quatre actions locales du lot M3 (jamais un
    /// fetch direct : le contrat « un seul fetch d'action » tient toujours).
    #[test]
    fn ecran_acces_branche_sur_les_actions_locales() {
        let bloc = bloc_acces();
        for action in [
            "access_list",
            "confirm_approve",
            "confirm_deny",
            "grant_revoke",
            "pair_approve",
            "pair_reject",
            "client_set_scopes",
            "client_revoke",
        ] {
            assert!(
                bloc.contains(&format!("action: '{action}'")),
                "action locale absente de l'écran Accès : {action}"
            );
            assert!(
                crate::manage::ACTIONS.contains(&action),
                "action absente du dispatcher : {action}"
            );
            assert!(
                crate::auth::action_reservee_admin(action),
                "action non réservée à la session locale : {action}"
            );
        }
        // trois boutons de décision pour une confirmation : une fois, permis, refus —
        // tous par la table ACT (plus aucun on*) et paramétrés par data-mode.
        for bouton in [
            "data-act=\"confirm-approve\" data-id=\"${esc(p.id)}\" data-mode=\"once\"",
            "data-act=\"confirm-approve\" data-id=\"${esc(p.id)}\" data-mode=\"window\"",
            "data-act=\"confirm-deny\" data-id=\"${esc(p.id)}\"",
        ] {
            assert!(
                bloc.contains(bouton),
                "bouton de décision manquant : {bouton}"
            );
        }
        // les identifiants de requête/permis viennent du moteur : ils ne
        // voyagent que dans data-id, échappés — jamais dans du code JS.
        assert!(
            !bloc.contains("js("),
            "l'écran Accès ne doit plus construire de littéral JS depuis une valeur reçue"
        );
        // le bandeau et la pastille lisent les compteurs de /api/state
        assert!(
            APP_JS.contains("state.access"),
            "le bandeau doit lire state.access"
        );
        assert!(APP_JS.contains("tbAccessN"), "pastille de compteur absente");
    }

    // ---------- Contrats Phase 3 : A11y, navigation & Top-Layer ----------

    #[test]
    fn coquille_a11y_et_navigation_clavier_verrouilles() {
        let html = INDEX_HTML.replace("\r\n", "\n");

        // 1. Lien d'évitement juste après <body>
        assert!(
            html.contains("<body>\n<a href='#content' class=\"skip-link\">"),
            "skip-link vers #content manquant juste après <body>"
        );

        // 2. Barre de titre : logo en <button> go-home, #discoverMenu non-draggable,
        //    aria-label sur chaque bouton interactif (hors items textuels du menu).
        assert!(
            html.contains(r#"<header id="titlebar" data-tauri-drag-region="deep">"#),
            "#titlebar doit garder data-tauri-drag-region=\"deep\""
        );
        assert!(
            html.contains(r#"<button type="button" class="logo" data-act="go-home""#),
            "le logo doit être un <button type=\"button\" data-act=\"go-home\">"
        );
        assert!(
            html.contains(r#"id="discoverMenu""#)
                && html.contains(r#"data-tauri-drag-region="false""#),
            "#discoverMenu doit porter data-tauri-drag-region=\"false\""
        );
        let tb_start = html.find("<header id=\"titlebar\"").unwrap();
        let tb_end = html[tb_start..].find("</header>").unwrap() + tb_start;
        let titlebar = &html[tb_start..tb_end];
        let mut scan = titlebar;
        while let Some(pos) = scan.find("<button") {
            let close = scan[pos..].find('>').unwrap() + pos;
            let tag = &scan[pos..=close];
            if !tag.contains(r#"role="menuitem""#) {
                assert!(
                    tag.contains("aria-label=\""),
                    "bouton de #titlebar sans aria-label (perdu au repli <= 1120 px) : {tag}"
                );
            }
            scan = &scan[close + 1..];
        }

        // 3. #accessBanner encapsulé dans un vrai <button>
        assert!(
            html.contains(
                r#"<div id="accessBanner" hidden aria-live="polite" role="status"><button type="button" id="accessBannerBtn" class="access-banner-btn" data-act="access-open""#
            ),
            "#accessBanner doit contenir #accessBannerBtn sans data-act sur le <div>"
        );

        // 4. Explorateur : aria-pressed, #hiddenBar en <button>, #localHead/.grp-toggle, #grip clavier
        assert!(
            html.contains(r#"data-act="exp-state" data-mode="all" aria-pressed="true""#),
            "pilule Tous doit porter aria-pressed=\"true\""
        );
        for mode in ["up", "down", "locked", "paused"] {
            assert!(
                html.contains(&format!(
                    r#"data-act="exp-state" data-mode="{mode}" aria-pressed="false""#
                )),
                "pilule {mode} doit porter aria-pressed=\"false\""
            );
        }
        assert!(
            html.contains(r#"<button type="button" id="hiddenBar" hidden data-act="show-hidden""#),
            "#hiddenBar doit être un <button type=\"button\">"
        );
        assert!(
            html.contains(
                r#"<div class="grphead" id="localHead"><button type="button" class="grp-toggle" data-act="group-toggle" data-key="local" aria-expanded="true" aria-controls="srvList""#
            ),
            "#localHead doit contenir un bouton .grp-toggle accessible"
        );
        for attr in [
            r#"id="grip""#,
            r#"tabindex="0""#,
            r#"aria-valuemin="180""#,
            r#"aria-valuemax="520""#,
            r#"aria-valuenow="320""#,
        ] {
            assert!(
                html.contains(attr),
                "attribut clavier manquant sur #grip : {attr}"
            );
        }

        // 5. Journal : #loghead sans data-act, #logToggleBtn, filtres, outils, #logbody aria-live="off"
        assert!(
            html.contains("<div id=\"loghead\">"),
            "#loghead ne doit plus porter data-act directement"
        );
        assert!(
            html.contains(
                r#"<button type="button" id="logToggleBtn" class="log-toggle" data-act="log-toggle" aria-expanded="false" aria-controls="logbody""#
            ),
            "#logToggleBtn manquant ou mal configuré"
        );
        for marker in [
            r#"id="logCountBadge""#,
            r#"id="logErrBadge""#,
            r#"id="logFilters""#,
            r#"data-act="log-filter" data-level="all" aria-pressed="true""#,
            r#"data-act="log-filter" data-level="warn" aria-pressed="false""#,
            r#"data-act="log-filter" data-level="error" aria-pressed="false""#,
            r#"data-act="log-copy""#,
            r#"data-act="log-clear""#,
            r#"id="logExpandBtn" class="log-tbtn" data-act="log-expand" aria-pressed="false""#,
            r#"<span class="tb-badge tb-hint">auto-d&eacute;tection : HTTP streamable &middot; stdio JSON-RPC</span>"#,
            r#"<div id="logbody" aria-live="off"></div>"#,
        ] {
            assert!(
                html.contains(marker),
                "marqueur journal manquant : {marker}"
            );
        }
        let st_auto_pos = html.find(r#"id="stAuto""#).unwrap();
        let st_auto_end = html[st_auto_pos..].find('>').unwrap() + st_auto_pos;
        assert!(
            html[st_auto_pos..=st_auto_end].contains("aria-label=\""),
            "#stAuto doit avoir un aria-label initial"
        );

        // 6. Dialogues, labels uniques, régions live et #toast popover="manual"
        assert!(
            html.contains(
                r#"<h2 id="dlgAddTitle"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d='M12 5v14M5 12h14'/></svg> Ajouter un serveur MCP</h2>"#
            ),
            "icône SVG manquante dans #dlgAddTitle"
        );
        assert!(
            html.contains(
                r#"id="storeSearchClear" class="search-clear" hidden data-act="store-clear-search""#
            ),
            "bouton clear recherche boutique manquant"
        );
        assert!(
            html.contains(r#"id="tDark" class="seg-btn" data-act="theme" data-theme="dark" aria-pressed="true""#)
                && html.contains(r#"id="tLight" class="seg-btn" data-act="theme" data-theme="light" aria-pressed="false""#),
            "aria-pressed initial manquant sur #tDark / #tLight"
        );
        assert!(
            !html.contains(r#"for="sNetEnabled""#) && !html.contains(r#"for="sNotifDesktop""#),
            "label dupliqué sur #sNetEnabled ou #sNotifDesktop"
        );
        assert!(
            html.contains(r#"id="lbl_net_enabled""#)
                && html.contains(
                    r#"id="sNetEnabled" role="switch" aria-labelledby="lbl_net_enabled""#
                )
                && html.contains(r#"id="lbl_notif_desktop""#)
                && html.contains(
                    r#"id="sNotifDesktop" role="switch" aria-labelledby="lbl_notif_desktop""#
                ),
            "aria-labelledby manquant sur #sNetEnabled ou #sNotifDesktop"
        );
        for live_id in [
            "importMsg",
            "machineMsg",
            "settingsMsg",
            "notifyMsg",
            "resetMsg",
            "runtimeMsg",
            "accessMsg",
            "loginMsg",
            "storeResults",
        ] {
            let p = html
                .find(&format!("id=\"{live_id}\""))
                .unwrap_or_else(|| panic!("élément live absent : {live_id}"));
            let e = html[p..].find('>').unwrap() + p;
            assert!(
                html[p..=e].contains(r#"aria-live="polite""#),
                "aria-live=\"polite\" manquant sur #{live_id}"
            );
        }
        assert!(
            html.contains(
                r#"<div id="toast" popover="manual" aria-live="polite" role="status"></div>"#
            ),
            "#toast doit utiliser popover=\"manual\" (Top Layer)"
        );
    }

    #[test]
    fn coquille_ids_uniques_et_sans_boutons_imbriques() {
        let mut ids = Vec::new();
        let bytes = INDEX_HTML.as_bytes();
        let mut i = 0usize;
        while let Some(rel) = INDEX_HTML[i..].find("id=\"") {
            let abs = i + rel;
            let prev = abs.checked_sub(1).and_then(|k| bytes.get(k)).copied();
            if matches!(prev, Some(b' ' | b'\n' | b'\r' | b'\t')) {
                let s = abs + 4;
                let e = INDEX_HTML[s..].find('"').map(|k| s + k).unwrap_or(s);
                let id = &INDEX_HTML[s..e];
                assert!(!ids.contains(&id), "id dupliqué dans INDEX_HTML : {id}");
                ids.push(id);
                i = e;
            } else {
                i = abs + 4;
            }
        }
        // Aucun <button> imbriqué dans un autre <button>
        let mut depth = 0usize;
        let mut pos = 0usize;
        while pos < bytes.len() {
            if bytes[pos..].starts_with(b"<button") {
                let next = bytes.get(pos + 7).copied().unwrap_or(b' ');
                if next == b' ' || next == b'>' || next == b'\n' || next == b'\r' {
                    assert_eq!(depth, 0, "bouton imbriqué détecté près de l'offset {pos}");
                    depth += 1;
                }
            } else if bytes[pos..].starts_with(b"</button>") {
                depth = depth.saturating_sub(1);
            }
            pos += 1;
        }
        assert_eq!(depth, 0, "balise <button> non fermée dans INDEX_HTML");
    }

    #[test]
    fn contrats_phase3_js_et_css() {
        assert!(
            APP_JS.contains("escPre"),
            "helper escPre() requis pour préserver les sauts de ligne JSON/args"
        );
        assert!(
            APP_JS.contains("showPopover"),
            "toast() doit promouvoir #toast via showPopover() en Top Layer"
        );
        assert!(
            !APP_JS.contains("'--key'"),
            "pairGuideHtml() ne doit plus générer '--key' (rejeté par bridge.rs)"
        );
        let table: Vec<String> = act_table().into_iter().map(|(k, _)| k).collect();
        for key in [
            "go-home",
            "log-filter",
            "log-copy",
            "log-clear",
            "log-expand",
            "store-clear-search",
            "store-reset-filters",
            "tool-search-clear",
            "tool-args-format",
            "copy-tool-res",
        ] {
            assert!(
                table.contains(&key.to_string()),
                "action Phase 3 absente de ACT : {key}"
            );
        }
        for sel in [
            ".skip-link",
            ".access-banner-btn",
            "#explorer.resizing",
            ".grp-toggle",
            ".crumb-link",
            ".d-head.st-DOWN",
            ".d-head.st-DISABLED",
            ".dash-link",
            ".t3",
            ".spin",
            ".search-clear",
            ".log-toggle",
            ".log-badge",
            ".log-filters",
            ".log-fbtn",
            ".log-tools",
            ".log-tbtn",
            ".tb-hint",
            ".logo-on-dark",
            ".logo-on-light",
            "#toast[popover]",
            r#".ctxmenu[data-x="2496"]"#,
            r#".ctxmenu[data-y="1376"]"#,
            ".acc-acts-sep",
            ".acc-guide-card",
            ".tile-neutral",
            ".dot.IDLE",
        ] {
            assert!(
                APP_CSS.contains(sel),
                "sélecteur Phase 3 attendu absent de app.css : {sel}"
            );
        }
    }

    #[test]
    fn boutique_bouton_supprimer_avec_confirmation() {
        let table: Vec<String> = act_table().into_iter().map(|(k, _)| k).collect();
        assert!(
            table.contains(&"uninstall-store".to_string()),
            "action uninstall-store absente de ACT"
        );
        assert!(
            APP_JS.contains("async function uninstallStore(")
                && APP_JS.contains("askConfirm(")
                && APP_JS.contains("action: 'uninstall_store'"),
            "uninstallStore() avec confirmation askConfirm et action uninstall_store requis"
        );
        assert!(
            APP_CSS.contains(".scard-inst-badge") && APP_CSS.contains(".btn-uninst"),
            "styles .scard-inst-badge et .btn-uninst requis dans app.css"
        );
    }

    #[test]
    fn bouton_accueil_barre_titre_ramene_au_dashboard_initial() {
        let html = INDEX_HTML.replace("\r\n", "\n");

        // 1. #tbExplToggle porte data-tauri-drag-region="false" et est suivi
        //    immédiatement par le bouton logo/accueil #tbHome (aucun autre <button entre eux).
        let expl_pos = html
            .find(r#"id="tbExplToggle""#)
            .expect("#tbExplToggle absent de INDEX_HTML");
        let expl_start = html[..expl_pos]
            .rfind("<button")
            .expect("balise ouvrante de #tbExplToggle absente");
        let expl_open_end = html[expl_start..]
            .find('>')
            .expect("fin de balise ouvrante #tbExplToggle absente")
            + expl_start;
        let expl_tag = &html[expl_start..=expl_open_end];
        assert!(
            expl_tag.contains(r#"data-tauri-drag-region="false""#),
            "#tbExplToggle doit porter data-tauri-drag-region=\"false\" : {expl_tag}"
        );

        let expl_close = html[expl_open_end..]
            .find("</button>")
            .expect("fermeture de #tbExplToggle absente")
            + expl_open_end
            + "</button>".len();
        let after_expl = &html[expl_close..];
        let next_btn = after_expl
            .find("<button")
            .expect("aucun <button après #tbExplToggle");
        let home_prefix = r#"<button type="button" class="logo" data-act="go-home" id="tbHome" data-tauri-drag-region="false""#;
        assert!(
            after_expl[next_btn..].starts_with(home_prefix),
            "#tbExplToggle doit être immédiatement suivi par #tbHome : {}",
            &after_expl[next_btn..after_expl.len().min(next_btn + 160)]
        );

        // 2. ACT mappe 'go-home' vers goHome(
        let table = act_table();
        let go_home_handler = table
            .iter()
            .find(|(k, _)| k == "go-home")
            .map(|(_, v)| v.as_str())
            .expect("action 'go-home' absente de ACT");
        assert!(
            go_home_handler.contains("goHome("),
            "ACT['go-home'] doit appeler goHome( : {go_home_handler}"
        );

        // 3. goHome() réinitialise complètement l'état vers le tableau de bord initial
        let go_home = fn_body(APP_JS, "goHome");
        for needle in [
            "selectedId = null",
            "detailCache = null",
            "detailTab = 'outils'",
            "overviewFilter = null",
            "expStateFilter = 'all'",
            "srvFilter",
            "expStatePills",
            "toolSearchQuery = ''",
            "closeDiscoverMenu(",
            "ctxClose(",
            "scrollTop = 0",
            "delete contentEl.dataset.srvId",
            "render()",
        ] {
            assert!(
                go_home.contains(needle),
                "goHome() doit contenir `{needle}`"
            );
        }

        // 4. select() délègue à goHome() quand id est vide
        let select_fn = fn_body(APP_JS, "select");
        assert!(
            select_fn.contains("goHome()"),
            "select() doit déléguer à goHome()"
        );

        // 5. CSS : curseur pointeur sur le logo et pointer-events:none sur ses enfants / icône
        assert!(
            APP_CSS.contains("cursor:pointer"),
            "app.css doit contenir cursor:pointer"
        );
        assert!(
            APP_CSS.contains("pointer-events:none"),
            "app.css doit contenir pointer-events:none"
        );
    }

    #[test]
    fn mode_hybride_bloque_config_stdio_directe_dans_conf_agent() {
        for (fname, bridge_tok, url_tok) in [
            ("hermesYaml", "'bridge'", "'--url'"),
            ("mcpJson", "'bridge'", "'--url'"),
            ("claudeCmd", "bridge", "--url"),
        ] {
            let body = fn_body(APP_JS, fname);
            let h_pos = body
                .find("hybridUrl(")
                .unwrap_or_else(|| panic!("{fname}() doit tester hybridUrl(s)"));
            let prog_pos = body.find("transport_info.program").unwrap_or_else(|| {
                panic!("{fname}() doit conserver le repli transport_info.program")
            });
            assert!(
                h_pos < prog_pos,
                "{fname}() doit tester hybridUrl(s) AVANT transport_info.program"
            );
            assert!(
                body.contains("mcp-control") && body.contains(bridge_tok) && body.contains(url_tok),
                "{fname}() doit générer le pont signé mcp-control bridge --url"
            );
        }

        let conf = fn_body(APP_JS, "confAgent");
        assert!(
            conf.contains("const hUrl = hybridUrl(s)"),
            "confAgent() doit lire hybridUrl(s)"
        );
        assert!(
            conf.contains("if (hUrl) {"),
            "confAgent() doit avoir une branche hybride exclusive `if (hUrl)`"
        );
        assert!(
            conf.contains("conf-hybrid-lock"),
            "confAgent() doit afficher le bandeau de blocage `.conf-hybrid-lock`"
        );
        assert!(
            conf.contains("Pont hybride Ed25519"),
            "confAgent() doit titrer les 3 blocs hybrides signés Ed25519"
        );
        assert!(
            !conf.contains("hybridBlk"),
            "confAgent() ne doit plus dupliquer le bloc JSON via hybridBlk"
        );
        assert!(
            APP_CSS.contains(".conf-note.conf-hybrid-lock"),
            "app.css doit styler `.conf-note.conf-hybrid-lock`"
        );
    }

    #[test]
    fn badge_docker_explorateur_et_detail_respectent_csp_et_xss() {
        let logo_line = APP_JS
            .lines()
            .find(|l| l.contains("const DOCKER_LOGO ="))
            .expect("DOCKER_LOGO doit être défini dans app.js");
        assert!(
            !logo_line.contains("style="),
            "DOCKER_LOGO ne doit contenir aucun style= (CSP stricte)"
        );
        let badge_fn = fn_body(APP_JS, "dockerBadge");
        assert!(
            !badge_fn.contains("style="),
            "dockerBadge() ne doit contenir aucun style= (CSP stricte)"
        );
        assert!(
            badge_fn.contains("esc(tip)") && badge_fn.contains("esc(lbl)"),
            "dockerBadge() doit échapper tip et lbl via esc()"
        );
        let row_fn = fn_body(APP_JS, "srvRow");
        assert!(
            row_fn.contains("dockerBadge(s, false)"),
            "srvRow() doit afficher dockerBadge(s, false) dans l'explorateur"
        );
        let det_fn = fn_body(APP_JS, "renderDetail");
        assert!(
            det_fn.contains("dockerBadge(s, true)"),
            "renderDetail() doit afficher dockerBadge(s, true) dans l'en-tête"
        );
        for cls in [".dockerwrap", ".dockerlogo", ".tag.docker"] {
            assert!(
                APP_CSS.contains(cls),
                "app.css doit définir la classe {cls}"
            );
        }
    }

    #[test]
    fn progression_mise_a_jour_en_direct_et_csp_stricte() {
        for needle in [
            r#"id="updateBanner""#,
            r#"id="updBannerProg""#,
            r#"id="updProgRow""#,
            r#"id="updProgBar""#,
            r#"data-act="upd-banner-close""#,
        ] {
            assert!(
                INDEX_HTML.contains(needle),
                "INDEX_HTML doit contenir `{needle}`"
            );
        }
        for needle in [
            "renderUpdateProgress",
            "fmtBytes",
            "dismissUpdateBanner",
            "POLL_UPDATE_MS",
            "update_status",
        ] {
            assert!(APP_JS.contains(needle), "APP_JS doit contenir `{needle}`");
        }
        for cls in ["#updateBanner", ".upd-prog"] {
            assert!(APP_CSS.contains(cls), "APP_CSS doit définir `{cls}`");
        }
        for fname in ["renderUpdateProgress", "setProgressBar"] {
            let body = fn_body(APP_JS, fname);
            assert!(
                !body.contains("style=") && !body.contains(".style."),
                "{fname}() ne doit contenir aucune mutation de style en ligne (CSP stricte)"
            );
        }
    }

    #[test]
    fn reglages_luminosite_et_couleur_texte_csp_stricte() {
        for needle in [
            r#"id="sTextBright""#,
            r#"id="sTextBrightVal""#,
            r#"data-act="text-bright""#,
            r#"data-act="text-color""#,
            r#"data-act="text-style-reset""#,
        ] {
            assert!(
                INDEX_HTML.contains(needle),
                "INDEX_HTML doit contenir `{needle}`"
            );
        }
        for color in [
            "default", "white", "warm", "amber", "emerald", "cyan", "blue", "violet", "rose",
            "lime",
        ] {
            assert!(
                INDEX_HTML.contains(&format!(r#"data-color="{color}""#)),
                "INDEX_HTML doit proposer la nuance `{color}`"
            );
        }
        for needle in [
            "applyTextStyle",
            "applyTextColorAndSave",
            "applyTextBrightAndSave",
            "resetTextStyle",
            "mcc.textColor",
            "mcc.textBright",
        ] {
            assert!(APP_JS.contains(needle), "APP_JS doit contenir `{needle}`");
        }
        for fname in [
            "applyTextStyle",
            "applyTextColorAndSave",
            "applyTextBrightAndSave",
            "resetTextStyle",
        ] {
            let body = fn_body(APP_JS, fname);
            assert!(
                !body.contains("style=") && !body.contains(".style."),
                "{fname}() ne doit contenir aucune mutation de style en ligne (CSP stricte)"
            );
        }
        assert!(
            APP_BOOT_JS.contains("mcc.textColor") && APP_BOOT_JS.contains("mcc.textBright"),
            "app-boot.js doit restaurer textColor et textBright avant le premier rendu"
        );
        for sel in [
            r#":root[data-text-bright]:not([data-text-bright="100"])"#,
            r#":root[data-text-color="emerald"]"#,
            r#":root[data-theme="light"][data-text-color="emerald"]"#,
        ] {
            assert!(APP_CSS.contains(sel), "APP_CSS doit définir `{sel}`");
        }
    }

    #[test]
    fn onglet_swarm_control_logo_explorateur_et_csp_stricte() {
        for needle in [
            r#"/swarm.css?v=__MCC_VERSION__"#,
            r#"/swarm.js?v=__MCC_VERSION__"#,
            r#"id="explTabMcp""#,
            r#"id="explTabSwarm""#,
            r#"data-act="expl-tab""#,
            r#"id="tbSwarmN""#,
            r#"data-mode="swarm""#,
            r#"/swarm-logo/logo.svg?v=__MCC_VERSION__"#,
        ] {
            assert!(
                INDEX_HTML.contains(needle),
                "INDEX_HTML doit contenir `{needle}`"
            );
        }
        assert!(
            !INDEX_HTML.contains(r#"id="tbSwarm""#),
            "Swarm ne doit plus être un bouton dans #titlebar mais un onglet à droite de Serveurs MCP"
        );
        assert!(
            APP_JS.contains("swarmLogoUrl")
                && APP_JS.contains("swarm_kind")
                && APP_JS.contains("swarmBadge"),
            "app.js doit afficher les logos officiels animés et le badge Swarm dans l'explorateur"
        );
        assert!(
            !SWARM_JS.contains("style=")
                && !SWARM_JS.contains(".style.")
                && !SWARM_JS.contains("fetch('/api/action'"),
            "swarm.js doit respecter la CSP stricte et passer uniquement par postAction()"
        );
        assert!(
            SWARM_CSS.contains(".srv-logo.swarm-logo")
                && SWARM_CSS.contains(".tag.swarm-res")
                && SWARM_CSS.contains(".expl-tab.sel"),
            "swarm.css doit définir les styles du logo frelon et de l'onglet Swarm"
        );
    }
}
