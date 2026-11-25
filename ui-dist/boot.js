/* Page de repli (frontendDist Tauri). Le core glisse du port 5090 jusqu'à +9 si occupé :
   on sonde la plage et on rejoint l'URL réelle au lieu d'un 5090 figé. */
(async () => {
  for (let p = 5090; p <= 5099; p++) {
    try {
      const r = await fetch('http://127.0.0.1:' + p + '/api/hello');
      if (r.ok) { location.replace('http://127.0.0.1:' + p + '/'); return; }
    } catch (e) { /* port suivant */ }
  }
  document.body.textContent = 'MCP Control introuvable sur les ports 5090-5099.';
})();
