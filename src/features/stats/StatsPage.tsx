import { PageHeader } from "../../components/PageHeader";
import { EmptyState } from "../../components/States";

export function StatsPage() {
  return (
    <section>
      <PageHeader title="Stats" description="Listen List and Collection, side by side." />
      <div className="split">
        <section className="section" aria-labelledby="stats-listen-list">
          <h2 id="stats-listen-list" className="section-title">
            Listen List
          </h2>
          <EmptyState
            icon="stats"
            title="No Listen List stats yet"
            description="Add albums to see decades, genres, and artists."
          />
        </section>
        <section className="section" aria-labelledby="stats-collection">
          <h2 id="stats-collection" className="section-title">
            Collection
          </h2>
          <EmptyState
            icon="stats"
            title="No Collection stats yet"
            description="Log listens and ratings to see your best years, genres, and artists."
          />
        </section>
      </div>
    </section>
  );
}
