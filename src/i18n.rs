//! Traductions : la clé est le texte anglais, la table donne le français.

use crate::store::Lang;

const FR: &[(&str, &str)] = &[
    (
        "The latest release is not signed.",
        "La dernière version publiée n'est pas signée.",
    ),
    (
        "Bad signature: update refused.",
        "Signature invalide : mise à jour refusée.",
    ),
    (
        "Waiting: it starts as soon as one of the running downloads finishes.",
        "En attente : il démarre dès qu'un des téléchargements en cours se termine.",
    ),
    ("Update the link", "Mettre à jour le lien"),
    (
        "This link has expired. Paste a new link to the same file (from the site): what is already downloaded is kept if the file is the same.",
        "Ce lien a expiré. Colle un nouveau lien vers le même fichier (depuis le site) : la partie déjà téléchargée est gardée si c'est le même fichier.",
    ),
    (
        "The link has expired or is no longer allowed.",
        "Le lien a expiré ou n'est plus autorisé.",
    ),
    (
        "The server sends the file compressed; it cannot be saved as is.",
        "Le serveur envoie le fichier compressé : impossible de l'enregistrer tel quel.",
    ),
    (
        "This drive (FAT32) cannot hold files of 4 GB or more.",
        "Ce disque (FAT32) ne peut pas contenir de fichiers de 4 Go ou plus.",
    ),
    (
        "Unsafe file name in torrent.",
        "Torrent refusé : un nom de fichier sortirait du dossier choisi.",
    ),
    (
        "Size not verifiable: the server did not announce it.",
        "Taille non vérifiable : le serveur ne l'a pas annoncée.",
    ),
    ("Cancel this download?", "Annuler ce téléchargement ?"),
    ("It leaves the list.", "Il sera retiré de la liste."),
    ("Also delete the partial file", "Supprimer aussi le fichier partiel"),
    ("Keep downloading", "Continuer"),
    ("Cancel download", "Annuler le téléchargement"),
    ("Partial file not found.", "Fichier partiel introuvable."),
    ("Start over", "Recommencer"),
    (
        "Not enough free disk space for this file.",
        "Pas assez de place sur le disque pour ce fichier.",
    ),
    ("retries, last error:", "essais, dernière erreur :"),
    (
        "Closed: the host refuses this extra connection.",
        "Fermée : l'hôte refuse cette connexion en plus.",
    ),
    (
        "The host refuses extra connections: connections closed automatically:",
        "L'hôte refuse les connexions en plus : connexions fermées automatiquement :",
    ),
    (
        "The .part file already shows its final size; disk space is only used as data arrives.",
        "Le fichier .part affiche déjà sa taille finale ; la place sur le disque n'est prise qu'au fil des données.",
    ),
    ("Done early? Take half.", "Fini tôt ? Prends la moitié."),
    (
        "A connection that finishes early takes half of the largest remaining range.",
        "Une connexion qui finit tôt prend la moitié du plus gros morceau restant.",
    ),
    ("Picks up where it stopped", "Reprend où il s'était arrêté"),
    (
        "After a crash, a power cut or a pause, a download resumes from where it was.",
        "Après un plantage, une coupure ou une pause, le téléchargement repart là où il en était.",
    ),
    (
        "Pair it in Settings, turn on auto-capture: browser downloads land here.",
        "Associe-la dans les Réglages, active la capture auto : tes téléchargements arrivent ici.",
    ),
    ("Videos too", "Les vidéos aussi"),
    (
        "Paste a video page link: yt-dlp fetches it in the quality you pick, or audio only.",
        "Colle le lien d'une page vidéo : yt-dlp la récupère dans la qualité choisie, ou le son seul.",
    ),
    ("Magnets and torrents", "Liens magnet et torrents"),
    (
        "Paste a magnet link or drop a .torrent file onto the window.",
        "Colle un lien magnet ou dépose un fichier .torrent sur la fenêtre.",
    ),
    ("Measure the gain", "Mesure le gain"),
    (
        "The Benchmark tab downloads the same link with 1 connection, then with N.",
        "L'onglet Comparatif télécharge le même lien avec 1 connexion, puis avec N.",
    ),
    ("Leave some bandwidth", "Garde de la bande passante"),
    (
        "Click the speed pill at the top: one limit for all downloads (or per download in its detail).",
        "Clique sur la pastille de débit en haut : une limite pour tous les téléchargements (ou par téléchargement dans son détail).",
    ),
    ("Always up to date", "Toujours à jour"),
    (
        "fluxdl tells you when a new version is out and updates itself in one click.",
        "fluxdl te prévient quand une nouvelle version sort et se met à jour en un clic.",
    ),
    ("Version", "Version"),
    ("Downloads", "Téléchargements"),
    ("Benchmark", "Comparatif"),
    ("Settings", "Réglages"),
    ("Paste a link…", "Colle un lien…"),
    ("Paste", "Coller"),
    ("Download", "Télécharger"),
    ("Choose folder", "Choisir le dossier"),
    ("Connections", "Connexions"),
    ("connections", "connexions"),
    ("Connections per download", "Connexions par téléchargement"),
    (
        "Optional checksum, verified after download.",
        "Somme de contrôle facultative, vérifiée après le téléchargement.",
    ),
    ("No downloads yet", "Aucun téléchargement"),
    (
        "Paste a link above. Each file is split across up to 32 parallel connections.",
        "Colle un lien ci-dessus. Chaque fichier est réparti sur jusqu'à 32 connexions en parallèle.",
    ),
    ("Pause", "Pause"),
    ("Resume", "Reprendre"),
    ("Cancel", "Annuler"),
    ("Open folder", "Ouvrir le dossier"),
    ("Remove from list", "Retirer de la liste"),
    ("Queued", "En attente"),
    ("Downloading", "En cours"),
    ("Paused", "En pause"),
    ("Completed", "Terminé"),
    ("Failed", "Échec"),
    ("Canceled", "Annulé"),
    ("left", "restant"),
    ("active", "actives"),
    ("Nothing selected", "Aucune sélection"),
    (
        "Pick a download to watch its connections at work.",
        "Choisis un téléchargement pour voir ses connexions travailler.",
    ),
    ("Download complete.", "Téléchargement terminé."),
    (
        "No live data: resume to see the connections.",
        "Pas de données en direct : reprends pour voir les connexions.",
    ),
    (
        "Remote file changed: restarted from zero.",
        "Le fichier distant a changé : repris de zéro.",
    ),
    (
        "Server refuses ranges: 1 connection.",
        "Le serveur refuse les plages : 1 connexion.",
    ),
    ("Speed", "Débit"),
    ("Progress", "Avancement"),
    ("Time left", "Reste"),
    ("steals", "vols"),
    ("Interface", "Interface"),
    ("About", "À propos"),
    ("of", "sur"),
    ("File map", "Carte du fichier"),
    ("stolen range", "morceau volé"),
    ("stolen", "volé"),
    (
        "Done early? Take half. This connection finished its own range and took half of the largest remaining one.",
        "Fini tôt ? Prends la moitié. Cette connexion a fini son morceau et a pris la moitié du plus gros restant.",
    ),
    ("peak", "pic"),
    (
        "Downloads the same link with 1 connection, then with N, to a temporary file deleted afterwards.",
        "Télécharge le même lien avec 1 connexion puis avec N, dans un fichier temporaire effacé ensuite.",
    ),
    ("Max seconds per run", "Secondes max par essai"),
    ("Run benchmark", "Lancer le comparatif"),
    ("Stop", "Arrêter"),
    ("Measuring with", "Mesure avec"),
    ("faster", "plus rapide"),
    ("with", "avec"),
    (
        "Little gain: the server or your line is probably already saturated by one connection.",
        "Peu de gain : le serveur ou ta ligne est sans doute déjà saturé par une seule connexion.",
    ),
    (
        "More connections help on fast lines and with slow servers.",
        "Plus de connexions aident sur une ligne rapide et avec un serveur lent.",
    ),
    ("Default folder", "Dossier par défaut"),
    ("New downloads are saved here.", "Les nouveaux téléchargements sont enregistrés ici."),
    ("Browse", "Parcourir"),
    ("Speed limit", "Limite de débit"),
    (
        "Shared by all downloads, also in the main window. Off means no limit.",
        "Commune à tous les téléchargements, aussi dans la fenêtre principale. Désactivée : aucune limite.",
    ),
    (
        "Speed limit for all downloads",
        "Limite de débit de tous les téléchargements",
    ),
    (
        "Unlimited",
        "Illimitée",
    ),
    (
        "Limit:",
        "Limite :",
    ),
    (
        "Custom",
        "Personnalisée",
    ),
    (
        "Starts at",
        "Démarre à",
    ),
    (
        "Downloads wait until then. Pause at",
        "Les téléchargements attendent jusque-là. Pause à",
    ),
    (
        "Downloads wait until then.",
        "Les téléchargements attendent jusque-là.",
    ),
    (
        "Start now",
        "Démarrer maintenant",
    ),
    (
        "Speed limit: torrents follow the global limit only (shared by all torrents).",
        "Limite de débit : les torrents suivent seulement la limite globale (commune à tous les torrents).",
    ),
    (
        "this download, on top of the global limit",
        "ce téléchargement, en plus de la limite globale",
    ),
    (
        "Downloaded by yt-dlp: a new limit applies at its next start (pause, then resume).",
        "Téléchargé par yt-dlp : une nouvelle limite s'applique à son prochain démarrage (pause, puis reprise).",
    ),
    (
        "Waiting for the scheduled start.",
        "En attente de l'heure de démarrage.",
    ),
    (
        "Scheduling",
        "Planification",
    ),
    (
        "Start the queue at",
        "Démarrer la file à",
    ),
    (
        "Downloads wait until this time (local time), then start.",
        "Les téléchargements attendent cette heure (heure locale), puis démarrent.",
    ),
    (
        "Pause at",
        "Pause à",
    ),
    (
        "Running downloads pause and wait for the next start time.",
        "Les téléchargements en cours se mettent en pause jusqu'à la prochaine heure de démarrage.",
    ),
    (
        "When everything is done",
        "Quand tout est fini",
    ),
    (
        "Sleep and shut down: 60 s countdown you can cancel. Never after a failed or paused download; not kept after a restart.",
        "Veille et arrêt : compte à rebours de 60 s, annulable. Jamais après un téléchargement en échec ou en pause ; pas gardé au prochain lancement.",
    ),
    (
        "Nothing",
        "Rien",
    ),
    (
        "Quit fluxdl",
        "Fermer fluxdl",
    ),
    (
        "Sleep",
        "Veille",
    ),
    (
        "Shut down",
        "Éteindre",
    ),
    (
        "The PC will shut down",
        "Le PC va s'éteindre",
    ),
    (
        "Shutting down in",
        "Arrêt dans",
    ),
    (
        "The PC will go to sleep",
        "Le PC va se mettre en veille",
    ),
    (
        "Going to sleep in",
        "Mise en veille dans",
    ),
    (
        "All downloads are finished.",
        "Tous les téléchargements sont finis.",
    ),
    ("Language", "Langue"),
    ("Applies immediately.", "S'applique tout de suite."),
    ("Reduce animations", "Réduire les animations"),
    (
        "No fades or movements. Also on when Windows animations are turned off.",
        "Ni fondus ni mouvements. Aussi actif quand les animations de Windows sont coupées.",
    ),
    ("MIT license, free and open source.", "licence MIT, libre et gratuit."),
    (
        "Up to 32 connections per file. Unfinished downloads resume where they stopped, even after a crash.",
        "Jusqu'à 32 connexions par fichier. Les téléchargements inachevés reprennent où ils se sont arrêtés, même après un plantage.",
    ),
    // v0.2 : mise à jour, extension, vidéos, torrents.
    ("Accept links from the browser", "Accepter les liens du navigateur"),
    ("Add a torrent", "Ajouter un torrent"),
    (
        "Asking peers for the torrent's file list…",
        "Demande de la liste des fichiers aux pairs…",
    ),
    ("Audio only", "Son seul"),
    ("Best quality", "Meilleure qualité"),
    ("Browser extension", "Extension du navigateur"),
    ("Cannot unpack ffmpeg.", "Impossible de décompresser ffmpeg."),
    ("Check for updates", "Rechercher des mises à jour"),
    ("Check now", "Vérifier"),
    ("Checking…", "Recherche…"),
    (
        "Checksum missing from the release.",
        "Empreinte absente de la version publiée.",
    ),
    ("Close", "Fermer"),
    ("Connected peers", "Pairs connectés"),
    ("Copy", "Copier"),
    (
        "Damaged file received (checksum differs): nothing was changed.",
        "Fichier reçu abîmé (empreinte différente) : rien n'a été changé.",
    ),
    ("Download a video", "Télécharger une vidéo"),
    (
        "Download link refused (outside GitHub).",
        "Lien de téléchargement refusé (hors de GitHub).",
    ),
    (
        "Downloaded by yt-dlp; picture and sound are merged by ffmpeg.",
        "Téléchargée par yt-dlp ; image et son sont réunis par ffmpeg.",
    ),
    ("Downloading and checking…", "Téléchargement et vérification…"),
    ("File missing from the release.", "Fichier absent de la version publiée."),
    (
        "GitHub is limiting requests for now: try again in an hour.",
        "GitHub limite les demandes pour l'instant : réessaie dans une heure.",
    ),
    ("GitHub returned an error.", "GitHub a répondu par une erreur."),
    (
        "GitHub unreachable: check your Internet connection.",
        "GitHub injoignable : vérifie la connexion à Internet.",
    ),
    ("GitHub answer interrupted.", "Réponse de GitHub interrompue."),
    ("Installed version", "Version installée"),
    ("Invalid published file.", "Fichier publié invalide."),
    ("Keep sharing after download", "Continuer à partager après le téléchargement"),
    ("Later", "Plus tard"),
    (
        "Local server on 127.0.0.1 only, for the fluxdl extension.",
        "Serveur local sur 127.0.0.1 seulement, pour l'extension fluxdl.",
    ),
    (
        "Looking up the latest yt-dlp, ffmpeg and deno…",
        "Recherche des dernières versions de yt-dlp, ffmpeg et deno…",
    ),
    (
        "Magnet links in your browser and .torrent files in Explorer will open here. You can change this later in Settings.",
        "Les liens magnet du navigateur et les fichiers .torrent de l'Explorateur s'ouvriront ici. Modifiable plus tard dans les Réglages.",
    ),
    ("New token (unpairs the extension)", "Nouveau jeton (désappaire l'extension)"),
    ("New version", "Nouvelle version"),
    ("No release published yet.", "Aucune version publiée pour l'instant."),
    ("Not now", "Pas maintenant"),
    (
        "Off: a torrent stops as soon as it is complete.",
        "Désactivé : un torrent s'arrête dès qu'il est complet.",
    ),
    ("Open a .torrent file", "Ouvrir un fichier .torrent"),
    ("Open magnet links and .torrent files", "Ouvrir les liens magnet et les .torrent"),
    ("Open torrents with fluxdl?", "Ouvrir les torrents avec fluxdl ?"),
    ("Pairing token", "Jeton d'appairage"),
    (
        "Paste it in the extension's options to pair it.",
        "À coller dans les options de l'extension pour l'appairer.",
    ),
    ("Peers", "Pairs"),
    ("peers", "pairs"),
    ("files", "fichiers"),
    ("Pieces", "Pièces"),
    ("Port", "Port"),
    ("Ratio", "Ratio"),
    ("ratio", "ratio"),
    ("Reading the available formats…", "Lecture des formats disponibles…"),
    ("Reading the torrent…", "Lecture du torrent…"),
    (
        "Registers fluxdl for your Windows account only.",
        "Associe fluxdl pour ton compte Windows seulement.",
    ),
    ("Restarting to install it…", "Redémarrage pour l'installer…"),
    (
        "Same port in the extension. Takes effect after a restart.",
        "Le même dans l'extension. Pris en compte au prochain lancement.",
    ),
    ("Seeding", "Partage"),
    ("Server not started:", "Serveur non démarré :"),
    ("Stopped.", "Arrêté."),
    (
        "The latest release has no Windows executable.",
        "La dernière version ne contient pas l'exécutable Windows.",
    ),
    ("Too many redirects: try again later.", "Trop de redirections : réessaie plus tard."),
    ("Torrents", "Torrents"),
    ("Unreadable answer from GitHub.", "Réponse de GitHub illisible."),
    (
        "Unreadable checksum: nothing was changed.",
        "Empreinte illisible : rien n'a été changé.",
    ),
    ("Update", "Mettre à jour"),
    ("Update and restart", "Mettre à jour et redémarrer"),
    ("Update failed", "Échec de la mise à jour"),
    ("Update now?", "Mettre à jour maintenant ?"),
    ("Update yt-dlp", "Mettre à jour yt-dlp"),
    ("Updates", "Mises à jour"),
    ("Upload", "Envoi"),
    ("Uploaded", "Envoyé"),
    ("Video tools needed", "Outils vidéo nécessaires"),
    ("Videos", "Vidéos"),
    (
        "Videos need yt-dlp (reads and downloads the video), ffmpeg (merges picture and sound) and deno (JavaScript required by YouTube). fluxdl can download the official builds from GitHub and verify their checksums:",
        "Les vidéos ont besoin de yt-dlp (lit et télécharge la vidéo), de ffmpeg (réunit image et son) et de deno (JavaScript exigé par YouTube). fluxdl peut télécharger les versions officielles depuis GitHub et vérifier leurs empreintes :",
    ),
    ("Windows only.", "Windows seulement."),
    ("Yes", "Oui"),
    ("You have", "Tu as la"),
    ("You have the latest version.", "Tu as la dernière version."),
    ("available", "disponible"),
    ("blocks", "cases"),
    ("downloaded by yt-dlp", "téléchargée par yt-dlp"),
    ("is ready", "est prête"),
    ("missing", "absent"),
    ("ready", "prêt"),
    ("up to", "jusqu'en"),
    ("Active downloads:", "Téléchargements en cours :"),
    (
        "They will pause, fluxdl restarts with the new version, and they resume on their own.",
        "Ils se mettent en pause, fluxdl redémarre avec la nouvelle version, puis ils reprennent tout seuls.",
    ),
    ("yt-dlp and ffmpeg", "yt-dlp et ffmpeg"),
    ("Try as video", "Essayer comme vidéo"),
    ("Open in browser", "Ouvrir dans le navigateur"),
    ("Watch the clipboard", "Surveiller le presse-papiers"),
    (
        "Offers to download a file link as soon as you copy it.",
        "Propose de télécharger un lien de fichier dès que tu le copies.",
    ),
    ("Link copied", "Lien copié"),
    ("Ignore", "Ignorer"),
    ("magnet link", "lien magnet"),
    ("video", "vidéo"),
    ("Drop to download", "Dépose pour télécharger"),
    (".torrent file or .url shortcut", "fichier .torrent ou raccourci .url"),
    ("Open the page", "Ouvrir la page"),
    ("This link is a web page", "Ce lien est une page web"),
    (
        "This link leads to a web page, not directly to a file. Open the page in your browser and click its download button: with the fluxdl extension (auto-capture on), the real file will arrive here.",
        "Ce lien mène à une page web, pas directement à un fichier. Ouvre la page dans ton navigateur et clique sur son bouton de téléchargement : avec l'extension fluxdl (capture auto activée), le vrai fichier arrivera ici.",
    ),
    (
        "This link is a web page, not a file.",
        "Ce lien mène à une page web, pas à un fichier.",
    ),
    (
        "yt-dlp not found: install yt-dlp and ffmpeg.",
        "yt-dlp introuvable : installe yt-dlp et ffmpeg.",
    ),
    (
        "No peer answered: torrent info not found.",
        "Aucun pair n'a répondu : infos du torrent introuvables.",
    ),
    ("Cannot write the file.", "Écriture du fichier impossible."),
    (
        "Cannot write the file (disk full?): nothing was changed.",
        "Écriture impossible (disque plein ?) : rien n'a été changé.",
    ),
    ("Download interrupted: try again.", "Téléchargement interrompu : réessaie."),
    ("Nothing to download.", "Rien à télécharger."),
    ("Cannot find the app's location.", "Emplacement de l'appli introuvable."),
    ("App file locked: nothing was changed.", "Fichier de l'appli verrouillé : rien n'a été changé."),
    ("Install the extension", "Installer l'extension"),
    (
        "Chrome, Edge, Brave, Firefox: guided, in 3 steps.",
        "Chrome, Edge, Brave, Firefox : guidé, en 3 étapes.",
    ),
    (
        "Without automatic pairing: paste it in the extension's options.",
        "Sans appairage automatique : colle-le dans les options de l'extension.",
    ),
    ("Install", "Installer"),
    ("Install the browser extension", "Installer l'extension du navigateur"),
    ("Add fluxdl to your browser", "Ajoute fluxdl à ton navigateur"),
    (
        "Right-click menu, capture of browser downloads.",
        "Menu du clic droit, capture des téléchargements.",
    ),
    ("Copy the path", "Copier le chemin"),
    ("Open", "Ouvrir"),
    ("Open the folder", "Ouvrir le dossier"),
    ("Opens", "Ouvre"),
    ("and copies the path.", "et copie le chemin."),
    (
        "Path copied. If the page did not open, type",
        "Chemin copié. Si la page ne s'est pas ouverte, tape",
    ),
    ("in the address bar.", "dans la barre d'adresse."),
    ("Cannot start", "Impossible de lancer"),
    (
        "Firefox not found: the button saves the zip in this folder; open about:debugging in Firefox.",
        "Firefox introuvable : le bouton enregistre le zip dans ce dossier ; ouvre about:debugging dans Firefox.",
    ),
    (
        "No browser found: open its extensions page yourself.",
        "Aucun navigateur trouvé : ouvre toi-même sa page des extensions.",
    ),
    ("Turn on “Developer mode”", "Active le « Mode développeur »"),
    (
        "Top right of the extensions page (Edge: bottom left).",
        "En haut à droite de la page des extensions (Edge : en bas à gauche).",
    ),
    ("Click “Load unpacked”", "Clique sur « Charger l'extension non empaquetée »"),
    (
        "The button appears under the page title.",
        "Le bouton apparaît sous le titre de la page.",
    ),
    ("Paste the path and confirm", "Colle le chemin et valide"),
    (
        "Ctrl+V in the folder picker (path already copied), then Select Folder.",
        "Ctrl+V dans la fenêtre de choix du dossier (chemin déjà copié), puis « Sélectionner un dossier ».",
    ),
    ("Open “This Firefox”", "Ouvre « Ce Firefox »"),
    (
        "On the left of the about:debugging page.",
        "À gauche de la page about:debugging.",
    ),
    (
        "Click “Load Temporary Add-on…”",
        "Clique sur « Charger un module complémentaire temporaire… »",
    ),
    (
        "The button is at the top of the page.",
        "Le bouton est en haut de la page.",
    ),
    ("Pick the zip file", "Choisis le fichier zip"),
    (
        "Ctrl+V pastes its path (already copied), then Open.",
        "Ctrl+V colle son chemin (déjà copié), puis « Ouvrir ».",
    ),
    (
        "Temporary add-on: Firefox removes it when it closes, until Mozilla signs the extension.",
        "Module temporaire : Firefox le retire à sa fermeture, tant que Mozilla n'a pas signé l'extension.",
    ),
    (
        "This folder is updated with fluxdl: after an update, click Reload on the extension's card.",
        "Ce dossier est mis à jour avec fluxdl : après une mise à jour, clique sur Recharger sur la carte de l'extension.",
    ),
    ("Developer mode", "Mode développeur"),
    ("Load unpacked", "Charger l'extension non empaquetée"),
    ("This Firefox", "Ce Firefox"),
    ("Load Temporary Add-on…", "Charger un module temporaire…"),
    ("Select Folder", "Sélectionner un dossier"),
    ("Connect the extension?", "Connecter l'extension ?"),
    (
        "A fluxdl extension asks for access",
        "Une extension fluxdl demande l'accès",
    ),
    ("origin", "origine"),
    ("Allow?", "Autoriser ?"),
    (
        "Allow only if you just installed it or clicked “Connect to fluxdl”. It will be able to send downloads to fluxdl.",
        "N'autorise que si tu viens de l'installer ou de cliquer sur « Connecter à fluxdl ». Elle pourra envoyer des téléchargements à fluxdl.",
    ),
    ("Deny", "Refuser"),
    ("Allow", "Autoriser"),
    ("Live: from its beginning", "Live depuis le début"),
    (
        "Off: recorded from now. Recording runs until the live ends or until Pause.",
        "Sinon : enregistré à partir de maintenant, jusqu'à la fin du live ou jusqu'à Pause.",
    ),
    (
        "Live: recorded from now until it ends or until Pause (this site cannot start from the beginning).",
        "Live : enregistré à partir de maintenant, jusqu'à sa fin ou jusqu'à Pause (ce site ne permet pas de partir du début).",
    ),
    ("Section only: from", "Seulement de"),
    ("to", "à"),
    ("Sections need ffmpeg.", "Un passage demande ffmpeg."),
    (
        "Cut at the nearest keyframe. A section is restarted, not resumed, after Pause.",
        "Coupé à l'image clé la plus proche. Après Pause, un passage recommence au début.",
    ),
    (
        "Times as hh:mm:ss, the end after the start.",
        "Heures au format hh:mm:ss, la fin après le début.",
    ),
    (
        "Empty: the whole video. Long videos can be paused and resumed.",
        "Vide : toute la vidéo. Une longue vidéo peut être mise en pause puis reprise.",
    ),
    (
        "With your login cookies for this site (members, subscribers): kept in memory only, given to yt-dlp in a private temporary file deleted right after.",
        "Avec tes cookies de connexion pour ce site (membres, abonnés) : gardés en mémoire seulement, donnés à yt-dlp dans un fichier temporaire privé effacé juste après.",
    ),
    ("Default video quality", "Qualité vidéo par défaut"),
    (
        "Pre-selected; used without asking while fluxdl is minimized.",
        "Cochée d'avance ; utilisée sans question quand fluxdl est réduit.",
    ),
    ("Best", "Meilleure"),
    ("original M4A/Opus", "original M4A/Opus"),
    ("Subtitles", "Sous-titres"),
    ("None for this video", "Aucun pour cette vidéo"),
    ("Embedded", "Intégrés"),
    (".srt file", "Fichier .srt"),
    ("Embed the thumbnail", "Miniature intégrée"),
    ("Embed the chapters", "Chapitres intégrés"),
    ("Format", "Format"),
    ("Original", "Original"),
    (
        "MP4: plays everywhere (H.264/AAC when available, up to 1080p on YouTube).",
        "MP4 : compatible partout (H.264/AAC si possible, 1080p au plus sur YouTube).",
    ),
    (
        "Original: best quality, MKV or WebM.",
        "Original : meilleure qualité, MKV ou WebM.",
    ),
    (
        "Subtitles, thumbnail and chapters need ffmpeg.",
        "Sous-titres, miniature et chapitres demandent ffmpeg.",
    ),
    (
        "In the interface language, when the video has it.",
        "Dans la langue de l'interface, si la vidéo l'a.",
    ),
    ("None", "Aucun"),
    (
        "Inside the video file (needs ffmpeg).",
        "Dans le fichier vidéo (ffmpeg requis).",
    ),
    ("Video format", "Format vidéo"),
    (
        "MP4 plays everywhere (H.264/AAC when available); Original keeps the best quality (MKV/WebM).",
        "MP4 : compatible partout (H.264/AAC si possible) ; Original : meilleure qualité (MKV/WebM).",
    ),
    (
        "Login cookies are gone (fluxdl was restarted): click the fluxdl button on the video page again to resume.",
        "Cookies de connexion perdus (fluxdl a redémarré) : clique à nouveau sur le bouton fluxdl de la page de la vidéo pour reprendre.",
    ),    (
        "This playlist is private: only its owner can see it.",
        "Cette playlist est privée : seul son propriétaire peut la voir.",
    ),
    (
        "Playlist or channel not found: check the link.",
        "Playlist ou chaîne introuvable : vérifie le lien.",
    ),
    ("This list has no video.", "Cette liste ne contient aucune vidéo."),
    ("This video is in a playlist", "Cette vidéo fait partie d'une playlist"),
    (
        "Download only this video, or choose among the videos of the whole playlist?",
        "Télécharger seulement cette vidéo, ou choisir parmi les vidéos de toute la playlist ?",
    ),
    ("This video only", "Cette vidéo seule"),
    ("The whole playlist", "Toute la playlist"),
    ("Download a playlist or channel", "Télécharger une playlist ou une chaîne"),
    ("Reading the list of videos…", "Lecture de la liste des vidéos…"),
    ("Selected:", "Sélectionnées :"),
    ("Check all", "Tout cocher"),
    ("Uncheck all", "Tout décocher"),
    ("Filter by title", "Filtrer par titre"),
    ("No video matches the filter.", "Aucune vidéo ne correspond au filtre."),
    ("Reading more videos…", "Lecture des vidéos suivantes…"),
    ("Load more", "Charger plus"),
    ("Quality", "Qualité"),
    ("Audio", "Son"),
    ("Subtitles in the interface language", "Sous-titres dans la langue de l'interface"),
    ("In a subfolder:", "Dans un sous-dossier :"),
    (
        "The selected videos are already in the list.",
        "Les vidéos cochées sont déjà dans la liste.",
    ),
    // Lot E : tri par type, extraction, empreinte.
    ("Music", "Musique"),
    ("Pictures", "Images"),
    ("Programs", "Programmes"),
    ("Other", "Autres"),
    ("After download", "Après le téléchargement"),
    ("Sort by file type", "Trier par type de fichier"),
    (
        "New files and videos go to the folder of their type. Torrents are not sorted.",
        "Les nouveaux fichiers et vidéos vont dans le dossier de leur type. Les torrents ne sont pas triés.",
    ),
    ("Extract archives automatically", "Extraire les archives automatiquement"),
    (
        "zip and 7z, into a folder named after the archive, next to it. RAR is not supported.",
        "zip et 7z, dans un dossier au nom de l'archive, à côté. RAR non pris en charge.",
    ),
    ("Delete the archive after extraction", "Supprimer l'archive après extraction"),
    (
        "Only once the extraction succeeded. Off: the archive is kept.",
        "Seulement si l'extraction a réussi. Désactivé : l'archive est gardée.",
    ),
    ("File", "Fichier"),
    ("Extract", "Extraire"),
    ("Compute SHA-256", "Calculer SHA-256"),
    ("Stop extracting", "Arrêter l'extraction"),
    ("Stop SHA-256", "Arrêter le SHA-256"),
    ("Extracting…", "Extraction…"),
    ("Extracted to:", "Extrait dans :"),
    ("Computing SHA-256…", "Calcul du SHA-256…"),
    ("Paste the expected SHA-256", "Colle le SHA-256 attendu"),
    ("Match", "Identique"),
    ("Mismatch", "Différent"),
    ("RAR archives are not supported.", "Archives RAR non prises en charge."),
    (
        "Only zip and 7z archives can be extracted.",
        "Seules les archives zip et 7z peuvent être extraites.",
    ),
    (
        "Archive refused: a file path would leave the folder (absolute or ..).",
        "Archive refusée : un chemin de fichier sortirait du dossier (absolu ou ..).",
    ),
    ("Encrypted archive: not supported.", "Archive chiffrée : non prise en charge."),
    (
        "Damaged archive: a file is larger than announced.",
        "Archive abîmée : un fichier est plus gros qu'annoncé.",
    ),
    ("Add", "Ajouter"),
    ("downloads?", "téléchargements ?"),
    (
        "Links sent from a web page by the browser extension. Those already in the list are skipped.",
        "Liens envoyés depuis une page web par l'extension du navigateur. Ceux déjà dans la liste sont ignorés.",
    ),
    ("History", "Historique"),
    ("Clear history", "Vider l'historique"),
    ("Clear history?", "Vider l'historique ?"),
    (
        "All entries leave the history. Downloaded files are kept.",
        "Toutes les entrées quittent l'historique. Les fichiers téléchargés sont gardés.",
    ),
    ("Search by name, link or site", "Rechercher par nom, lien ou site"),
    ("All", "Tous"),
    ("Successful", "Terminés"),
    ("Errors", "Échoués"),
    ("All types", "Tous les types"),
    ("Newest", "Récents"),
    ("Oldest", "Anciens"),
    ("No history yet", "Historique vide"),
    (
        "Finished, failed and canceled downloads appear here, even after they leave the list.",
        "Les téléchargements terminés, échoués et annulés s'affichent ici, même retirés de la liste.",
    ),
    ("Nothing matches", "Aucun résultat"),
    ("Change the search or the filters.", "Change la recherche ou les filtres."),
    ("Retry", "Réessayer"),
    ("Download again", "Retélécharger"),
    ("Open file", "Ouvrir le fichier"),
    ("File not found", "Fichier introuvable"),
    ("Copy link", "Copier le lien"),
    ("Remove from history", "Retirer de l'historique"),
    ("File not found on the server", "Fichier introuvable sur le serveur"),
    (
        "The server is having trouble: try again later",
        "Le serveur est en panne : réessaie plus tard",
    ),
    (
        "The server limits requests: try again later",
        "Le serveur limite les demandes : réessaie plus tard",
    ),
    ("The server refuses access", "Le serveur refuse l'accès"),
    ("The server answered with an error", "Le serveur a répondu par une erreur"),
    (
        "Server unreachable: check the connection or try again later.",
        "Serveur injoignable : vérifie la connexion ou réessaie plus tard.",
    ),
    (
        "Assembling the video failed (ffmpeg).",
        "L'assemblage de la vidéo a échoué (ffmpeg).",
    ),
    ("ffmpeg is missing for this video.", "ffmpeg manque pour cette vidéo."),
    ("The download failed.", "Le téléchargement a échoué."),
    (
        "The PC will shut down when everything is finished",
        "Le PC s'éteindra quand tout sera fini",
    ),
    (
        "The PC will go to sleep when everything is finished",
        "Le PC se mettra en veille quand tout sera fini",
    ),
    ("Disable", "Désactiver"),
    ("Back to the list", "Retour à la liste"),
    ("Starts tomorrow at", "Démarre demain à"),
    ("Remove from list (the file stays)", "Retirer de la liste (le fichier reste)"),
    (
        "MP4 unavailable for this video (no H.264): original format, MKV or WebM.",
        "MP4 impossible pour cette vidéo (pas de H.264) : format d'origine, MKV ou WebM.",
    ),
    ("No thumbnail for this video.", "Pas de miniature pour cette vidéo."),
    (
        "ffmpeg missing: a single file with picture and sound (quality may be lower).",
        "ffmpeg absent : un seul fichier image et son (qualité parfois moindre).",
    ),
    ("Already in the list.", "Déjà dans la liste."),
    ("Already downloaded.", "Déjà téléchargé."),
    ("Retrying the failed download.", "Le téléchargement en échec est relancé."),
    (
        "Paste an http(s) or magnet link.",
        "Colle un lien http(s) ou magnet.",
    ),
    (
        "The clipboard has no link.",
        "Le presse-papiers ne contient pas de lien.",
    ),
    (
        "This video is for subscribers of the channel only.",
        "Vidéo réservée aux abonnés de la chaîne.",
    ),
    (
        "Video kept, but subtitles, thumbnail or chapters could not be added.",
        "Vidéo gardée, mais sous-titres, miniature ou chapitres n'ont pas pu être ajoutés.",
    ),
];

pub fn t(lang: Lang, en: &'static str) -> &'static str {
    tr(lang, en)
}

/// Comme `t`, pour un texte non statique (message d'erreur venu d'un fil) : traduit s'il est
/// dans la table, sinon rendu tel quel.
pub fn tr(lang: Lang, en: &str) -> &str {
    match lang {
        Lang::En => en,
        Lang::Fr => FR.iter().find(|(k, _)| *k == en).map_or(en, |(_, f)| f),
    }
}

/// Message d'erreur lisible et traduit, avec le détail technique (message brut) quand il n'est
/// pas déjà une phrase de la table : codes HTTP, réseau, ffmpeg, message brut de yt-dlp.
pub fn erreur(lang: Lang, m: &str) -> (String, Option<String>) {
    if FR.iter().any(|(k, _)| *k == m) {
        return (tr(lang, m).to_string(), None);
    }
    let bas = m.to_ascii_lowercase();
    let code = bas
        .split("http ")
        .nth(1)
        .and_then(|r| r.get(..3))
        .and_then(|c| c.parse::<u16>().ok());
    let court = match code {
        Some(404) => format!("{} (404).", t(lang, "File not found on the server")),
        Some(429) => format!(
            "{} (429).",
            t(lang, "The server limits requests: try again later")
        ),
        Some(c @ 500..=599) => format!(
            "{} ({c}).",
            t(lang, "The server is having trouble: try again later")
        ),
        Some(c @ (401 | 403)) => format!("{} ({c}).", t(lang, "The server refuses access")),
        Some(c) => format!("{} ({c}).", t(lang, "The server answered with an error")),
        None if [
            "timeout",
            "dns",
            "connection failed",
            "connection refused",
            "network",
        ]
        .iter()
        .any(|k| bas.contains(k)) =>
        {
            t(
                lang,
                "Server unreachable: check the connection or try again later.",
            )
            .into()
        }
        None if bas == "ffmpeg missing" => t(lang, "ffmpeg is missing for this video.").into(),
        None if bas.contains("ffmpeg") => t(lang, "Assembling the video failed (ffmpeg).").into(),
        None => t(lang, "The download failed.").into(),
    };
    (court, Some(m.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erreurs_lisibles() {
        let fr = |m: &str| erreur(Lang::Fr, m);
        assert_eq!(
            fr("probe: HTTP 404"),
            (
                "Fichier introuvable sur le serveur (404).".into(),
                Some("probe: HTTP 404".into())
            )
        );
        assert!(fr("HTTP 503").0.starts_with("Le serveur est en panne"));
        assert!(fr("probe: Dns Failed").0.starts_with("Serveur injoignable"));
        assert!(fr("timeout").0.starts_with("Serveur injoignable"));
        assert_eq!(
            fr("ffmpeg exited with code -11").0,
            "L'assemblage de la vidéo a échoué (ffmpeg)."
        );
        assert_eq!(fr("[youtube] x: boom").0, "Le téléchargement a échoué.");
        // Phrase déjà dans la table : traduite, sans détail.
        assert_eq!(
            fr(crate::engine::ERR_EXPIRE),
            (tr(Lang::Fr, crate::engine::ERR_EXPIRE).into(), None)
        );
        assert_eq!(
            erreur(Lang::En, "HTTP 410").0,
            "The server answered with an error (410)."
        );
    }
}
