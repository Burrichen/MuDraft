import { LibraryView } from "../LibraryView";

export function CollectionPage() {
  return (
    <LibraryView
      source="collection"
      title="Collection"
      description="Albums you've listened to or added directly."
      emptyTitle="Your Collection is empty"
      emptyDescription="Albums appear here once you log a listen or add them to your collection."
    />
  );
}
