import { LibraryView } from "../LibraryView";

export function ListenListPage() {
  return (
    <LibraryView
      source="listen_list"
      title="Listen List"
      description="Albums you want to hear."
      emptyTitle="Your Listen List is empty"
      emptyDescription="Search MusicBrainz, enter an album by hand, or import a CSV to get started."
    />
  );
}
