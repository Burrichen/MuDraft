/** Data-source acknowledgements. */

export function DataSources() {
  return (
    <section className="panel section" aria-labelledby="settings-sources">
      <h2 id="settings-sources" className="section-title">
        Data sources
      </h2>
      <ul className="settings-sources">
        <li>
          <strong>MusicBrainz</strong> (musicbrainz.org) — album, edition, track, and artist data,
          made available by the MetaBrainz Foundation under CC0. Requests are limited to one per
          second, as MusicBrainz asks.
        </li>
        <li>
          <strong>Cover Art Archive</strong> (coverartarchive.org) — album artwork, downloaded only
          with your permission. Images belong to their respective rights holders.
        </li>
        <li>
          Broad genres are MuDraft’s own grouping of MusicBrainz genres and tags; you can change an
          album’s genres on its page.
        </li>
      </ul>
    </section>
  );
}
