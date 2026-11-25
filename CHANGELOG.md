# Changelog

Toutes les évolutions notables d'`audiok`.


## [0.3.0] - 2026-11-25

### Corrigé
- Écho sur Discord en réception vocale quand Telegram passait en grâce : l'override d'entrée VAC suit maintenant la durée de vie de la redirection (PID vivant, source présente ou encore en grâce et redirigée) au lieu de rester collé au profil.
- Bascule bypass : les entrées VAC sont restaurées/réappliquées immédiatement, même pour un aller-retour très rapide.
- Clés de profil `processus <pid>` : elles ne sont plus persistées et sont retirées des sorties, `mix_gains`, affectations VAC et alias à l'assainissement et à la sauvegarde.

## [0.2.0] - 2026-11-23

### Ajouté
- Écoute CUE/Mixe repensée : un bouton par sortie routée (hors sortie CUE), surbrillance violette du mixe actif, sélection persistée. Un mixe reste toujours sélectionné : auto-sélection du premier, remplacement automatique si l'actif disparaît, clic sur l'actif sans désélection.
- Le fader CUE édite le mixe sélectionné même en mode PFL.
- Sélecteur de langue compact (45 px) : code à deux lettres (`FR`, `EN`…) ou `🌐` pour Auto ; menu déroulant complet (drapeau + code + nom).
- Instance unique : une seconde instance est refusée avec un message localisé.
- Script `build_release.cmd` produisant `audiok_<version>_win_x64.exe`.
- Licence MIT (`LICENSE.md`) et section Licence dans le README.

### Modifié
- Panneau CUE : boutons de mixe en haut, dans l'ordre d'apparition ; bloc bas réordonné en `PFL/MIXE` → sélecteur CUE → indicateur de niveau pleine largeur → fader + vumètre. `Reset mixe` supprimé.
- i18n : clés obsolètes retirées, textes CUE ajoutés dans les 8 tables (FR, EN, DE, ES, IT, PT, ZH, JA) ; message « déjà lancé » adouci et traductions alignées.
- README : exécutable versionné, captures d'écran, licence.

### Corrigé
- Démarrage avec Windows / `--tray` : la fenêtre n'est plus brièvement visible au lancement.
- Sorties matérielles absentes : elles restent sélectionnées et se rebindent automatiquement au retour, sans réassignation manuelle.
- Période de grâce : une application temporairement absente mais dont le processus est encore vivant reste redirigée (`protected_redirect_pids`), évitant les allers-retours de routage et les doublons.
- Warning Clippy `outputs` inutilisé supprimé.


## [0.1.0] - 2026-11-21

Première version publiable : gestionnaire audio Windows 10/11 en Rust (eframe/egui).

### Fonctionnalités
- Capture des micros et des sessions d'applications, routage vers une ou plusieurs sorties (matériel ou application) via WASAPI ; support Virtual Audio Cable (VAC, 2 câbles) et bypass.
- Tranches : volume `-24…+24 dB`, trim/gain, retard `0…5000 ms`, panoramique `G`/`D`, réduction de bruit RNNoise (micros), mute, solo et PFL.
- Master : bypass, mutes (micros, entrées, sorties, global), bus CUE/PFL avec modes Casque (pré-fader) et Mixe (post-fader).
- Profils : routages, volumes, apparences et raccourcis, sauvegarde automatique après 2 s d'inactivité.
- Raccourcis globaux `F9` (micros), `F10` (global) et `Ctrl+Maj+1…9/0` (profils).
- 8 langues + Auto : français, anglais, allemand, espagnol, italien, portugais, chinois, japonais.
- Icône systray : afficher/masquer, quitter, démarrage avec Windows ; apparence personnalisable (couleurs, image de fond, voile).
- Journal rotatif (2 Mo) dans `%LOCALAPPDATA%\audiok`, configuration et profils au même endroit.
- Binaire autonome x64 et README.
