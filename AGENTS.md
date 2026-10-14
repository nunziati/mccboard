# mccboard

**Multi Claude Code dashBoard**: un centro di controllo per più istanze e account di Claude Code. Comprende:
- widget agganciato alla barra delle applicazioni e icona nella tray;
- dashboard;
- avvio di profili affiancati dell'app desktop;
- setup guidato delle macchine remote via SSH.

Progetto personale di Giacomo. La gestione (contesto, decisioni, roadmap, ricerche) sta nella repo privata `nunziati/claude-account-switcher-project`:
- `CONTEXT.md`: decisioni D001…, in particolare D005–D012;
- `docs/2026-10-14-centro-di-controllo.md`: architettura e fasi;
- `research/`: widget esistenti, claude-swap, multi-claude, funzionamento di Claude Code.

Prima di cambiare direzione su qualcosa di architetturale, controlla lì.

## Architettura in breve
- **Un'app sola**, sulla macchina principale (oggi Windows). Niente server e niente agenti installati altrove.
- **Macchine remote opzionali**, raggiunte con l'`ssh` di sistema eseguendo piccoli script portati dall'app. Sul remoto non gira nessun servizio.
- **Multi-account opzionale.** Modello:
  - *account*: i consumi sono per account;
  - *istanza*: account + cartella di configurazione + macchina;
  - *profilo desktop*: istanza dell'app Claude desktop con `--user-data-dir` proprio.
- **Consumi**: prima i dati delle sessioni (statusline catturata per istanza), altrimenti `GET https://api.anthropic.com/api/oauth/usage`, una volta per account.

## Regole sui token: non negoziabili
1. L'access token OAuth si **legge soltanto**, da `<config dir>/.credentials.json`.
2. **Mai rinnovarlo** (niente chiamate a `oauth/token`) e **mai scrivere** il file delle credenziali. Rinnovarlo cambierebbe il refresh token e Claude Code perderebbe il login. Token scaduto = stato "scaduto": lo rinnova Claude Code.
3. **Un token non lascia la macchina** su cui sta. Per un'istanza remota la chiamata all'endpoint la esegue il remoto via SSH, e torna solo il risultato.
4. **Endpoint dei consumi.** Ha un budget basso ed è condiviso con Claude Code:
   - al massimo una richiesta ogni 60 s per account;
   - dopo un 429 o un errore l'intervallo raddoppia, fino a 300 s;
   - si mostrano gli ultimi valori noti;
   - una finestra si azzera quando passa il suo `resets_at`.
5. **Mai fare `/logout`**, e mai proporlo per cambiare account: revoca il token. Per un altro account si usa un'altra cartella di configurazione.
6. **Mai stampare, loggare o salvare** token o credenziali, nemmeno nei test o negli errori.
7. I file di mccboard stanno in `~/.mccboard/`, **mai dentro `~/.claude`**.

## Stack
Tauri v2: core in Rust, interfaccia web. Il codice specifico per sistema operativo (aggancio alla barra, ID nella barra di Windows, icone) sta in moduli separati per OS.

## Convenzioni
- La lingua dei documenti di progetto è l'italiano.
- Commit piccoli e descrittivi.
