import type { DocsLabels } from '@masmarino/gabarit/docs'
import { Language } from '../shared/i18n/languages'

const FR: DocsLabels = {
  documentation: 'Documentation',
  showContents: 'Afficher le sommaire',
  hideContents: 'Masquer le sommaire',
  searchLabel: 'Rechercher dans la documentation',
  searchPlaceholder: 'Rechercher…',
  searching: 'Recherche…',
  noResults: 'Aucune page ne correspond.',
  searchFailed: 'La documentation n’a pas pu être chargée.',
  results: (count) =>
    count === 0 ? 'Aucun résultat' : count === 1 ? '1 résultat' : `${count} résultats`,
  onThisPage: 'Sur cette page',
  breadcrumb: 'Fil d’Ariane',
  neighbours: 'Pages voisines',
  previous: 'Précédent',
  next: 'Suivant',
  notFoundHeading: 'Page introuvable',
  notFoundMessage:
    'Cette page n’existe pas dans la documentation. Vérifiez l’adresse, ou repartez du début.',
  toStart: 'Aller au début de la documentation',
  loadFailed: 'La documentation n’a pas pu être chargée.',
  retry: 'Réessayer',
  loading: 'Chargement de la page…',
  codeBlock: 'Bloc de code',
  table: 'Tableau',
}

const ES: DocsLabels = {
  documentation: 'Documentación',
  showContents: 'Mostrar el índice',
  hideContents: 'Ocultar el índice',
  searchLabel: 'Buscar en la documentación',
  searchPlaceholder: 'Buscar…',
  searching: 'Buscando…',
  noResults: 'Ninguna página coincide.',
  searchFailed: 'No se ha podido cargar la documentación.',
  results: (count) =>
    count === 0 ? 'Ningún resultado' : count === 1 ? '1 resultado' : `${count} resultados`,
  onThisPage: 'En esta página',
  breadcrumb: 'Ruta de navegación',
  neighbours: 'Páginas vecinas',
  previous: 'Anterior',
  next: 'Siguiente',
  notFoundHeading: 'Página no encontrada',
  notFoundMessage:
    'Esta página no existe en la documentación. Compruebe la dirección o vuelva al principio.',
  toStart: 'Ir al principio de la documentación',
  loadFailed: 'No se ha podido cargar la documentación.',
  retry: 'Reintentar',
  loading: 'Cargando la página…',
  codeBlock: 'Bloque de código',
  table: 'Tabla',
}

const DE: DocsLabels = {
  documentation: 'Dokumentation',
  showContents: 'Inhaltsverzeichnis anzeigen',
  hideContents: 'Inhaltsverzeichnis ausblenden',
  searchLabel: 'In der Dokumentation suchen',
  searchPlaceholder: 'Suchen…',
  searching: 'Wird gesucht…',
  noResults: 'Keine Seite passt.',
  searchFailed: 'Die Dokumentation konnte nicht geladen werden.',
  results: (count) =>
    count === 0 ? 'Keine Ergebnisse' : count === 1 ? '1 Ergebnis' : `${count} Ergebnisse`,
  onThisPage: 'Auf dieser Seite',
  breadcrumb: 'Brotkrumennavigation',
  neighbours: 'Benachbarte Seiten',
  previous: 'Zurück',
  next: 'Weiter',
  notFoundHeading: 'Seite nicht gefunden',
  notFoundMessage:
    'Diese Seite gibt es in der Dokumentation nicht. Prüfen Sie die Adresse oder beginnen Sie von vorn.',
  toStart: 'Zum Anfang der Dokumentation',
  loadFailed: 'Die Dokumentation konnte nicht geladen werden.',
  retry: 'Erneut versuchen',
  loading: 'Seite wird geladen…',
  codeBlock: 'Codeblock',
  table: 'Tabelle',
}

const IT: DocsLabels = {
  documentation: 'Documentazione',
  showContents: 'Mostra il sommario',
  hideContents: 'Nascondi il sommario',
  searchLabel: 'Cerca nella documentazione',
  searchPlaceholder: 'Cerca…',
  searching: 'Ricerca in corso…',
  noResults: 'Nessuna pagina corrisponde.',
  searchFailed: 'Impossibile caricare la documentazione.',
  results: (count) =>
    count === 0 ? 'Nessun risultato' : count === 1 ? '1 risultato' : `${count} risultati`,
  onThisPage: 'In questa pagina',
  breadcrumb: 'Percorso di navigazione',
  neighbours: 'Pagine vicine',
  previous: 'Precedente',
  next: 'Successivo',
  notFoundHeading: 'Pagina non trovata',
  notFoundMessage:
    'Questa pagina non esiste nella documentazione. Controlla l’indirizzo o ricomincia dall’inizio.',
  toStart: 'Vai all’inizio della documentazione',
  loadFailed: 'Impossibile caricare la documentazione.',
  retry: 'Riprova',
  loading: 'Caricamento della pagina…',
  codeBlock: 'Blocco di codice',
  table: 'Tabella',
}

/** The reader's strings in the interface's language; the documentation itself is written in French. */
export function docsLabelsFor(language: Language): Partial<DocsLabels> {
  return { en: {}, fr: FR, es: ES, de: DE, it: IT }[language]
}
