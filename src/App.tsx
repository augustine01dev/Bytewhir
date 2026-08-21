import React, { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./App.css";

export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: string | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error) {
    return { error: error.message };
  }

  render() {
    if (this.state.error) {
      return (
        <div className="error-screen">
          <div className="error-card">
            <h2>⚠️ App Error</h2>
            <pre>{this.state.error}</pre>
          </div>
        </div>
      );
    }

    return this.props.children;
  }
}

interface DigestItem {
  id: string;
  source: string;
  category: string | null;
  title: string;
  summary: string | null;
  thumbnail_url: string | null;
  url: string;
  published_at: string;
  fetched_at: string;
  seen: number;
  starred: number;
}

interface SourceStyle {
  color: string;
  icon: string;
}

interface Source {
  id: string;
  name: string;
  enabled: number;
}

type AppView = "feed" | "saved" | "sources";

const FILTERS = [
  { value: "All", label: "All Streams" },
  { value: "AI", label: "Artificial Intelligence" },
  { value: "Tech", label: "Tech & Web" },
  { value: "New releases", label: "New Releases" },
  { value: "Updates", label: "Updates" },
];

function App() {
  const digestStaleAfterMs = 3 * 60 * 60 * 1000;
  const [items, setItems] = useState<DigestItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [error, setError] = useState("");
  const [activeFilter, setActiveFilter] = useState("All");
  const [activeView, setActiveView] = useState<AppView>("feed");
  const [sources, setSources] = useState<Source[]>([]);
  const [sourcesLoading, setSourcesLoading] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [failedImageIds, setFailedImageIds] = useState<Set<string>>(() => new Set());
  const startupFetchStarted = useRef(false);
  const initializationComplete = useRef(false);

  const handleImageError = (id: string) => {
    setFailedImageIds((current) => {
      if (current.has(id)) {
        return current;
      }

      const next = new Set(current);
      next.add(id);
      return next;
    });
  };

  const loadFeed = async (append = false) => {
    if (append) {
      setLoadingOlder(true);
    } else {
      setLoading(true);
    }
    setError("");

    try {
      let beforeTimestamp: string | null = null;
      if (append && items.length > 0) {
        beforeTimestamp = activeView === "saved"
          ? items[items.length - 1].fetched_at
          : items[items.length - 1].published_at;
      }

      const data: DigestItem[] = activeView === "saved"
        ? await invoke("get_saved_digest", { beforeTimestamp, limit: 20 })
        : await invoke("get_digest", {
            category: activeFilter === "All" ? null : activeFilter,
            beforeTimestamp,
            limit: 20,
          });

      if (append) {
        setItems((previous) => [...previous, ...data]);
      } else {
        setItems(data);
      }

      const unseenIds = data.filter((item) => item.seen === 0).map((item) => item.id);
      if (unseenIds.length > 0) {
        await invoke("mark_seen", { ids: unseenIds });
      }
    } catch (loadError: any) {
      setError(loadError.toString());
    } finally {
      if (append) {
        setLoadingOlder(false);
      } else {
        setLoading(false);
      }
    }
  };

  const loadSources = async () => {
    setSourcesLoading(true);
    setError("");
    try {
      const data: Source[] = await invoke("get_sources");
      setSources(data);
    } catch (sourceError: any) {
      setError(sourceError.toString());
    } finally {
      setSourcesLoading(false);
    }
  };

  const fetchNetworkDigest = async () => {
    setLoading(true);
    setError("");

    try {
      await invoke("fetch_digest");
      await loadFeed(false);
    } catch (fetchError: any) {
      setError(fetchError.toString());
      setLoading(false);
    }
  };

  useEffect(() => {
    const initialize = async () => {
      await loadFeed(false);

      try {
        const latestFetchedAt = await invoke<string | null>("get_latest_fetched_at");
        const latestFetchedTime = latestFetchedAt ? Date.parse(latestFetchedAt) : Number.NaN;
        const isStale =
          !Number.isFinite(latestFetchedTime) ||
          Date.now() - latestFetchedTime > digestStaleAfterMs;

        if (isStale && !startupFetchStarted.current) {
          startupFetchStarted.current = true;
          void fetchNetworkDigest();
        }
      } catch (startupError: any) {
        setError(startupError.toString());
      } finally {
        initializationComplete.current = true;
      }
    };

    void initialize();
  }, []);

  useEffect(() => {
    if (!initializationComplete.current) {
      return;
    }

    setExpandedId(null);
    if (activeView === "sources") {
      void loadSources();
    } else {
      void loadFeed(false);
    }
  }, [activeFilter, activeView]);

  const getSourceStyle = (source: string): SourceStyle => {
    switch (source.toLowerCase()) {
      case "github":
        return { color: "#a78bfa", icon: "code" };
      case "hacker news":
      case "hn":
        return { color: "#ff8a3d", icon: "forum" };
      case "arxiv":
        return { color: "#ff7c88", icon: "science" };
      case "techcrunch":
        return { color: "#65d6b0", icon: "rss_feed" };
      default:
        return { color: "#aeb7c8", icon: "newspaper" };
    }
  };

  const getSourceClass = (source: string) => {
    switch (source.toLowerCase()) {
      case "github":
        return "github";
      case "hacker news":
      case "hn":
        return "hn";
      case "arxiv":
        return "arxiv";
      case "techcrunch":
        return "techcrunch";
      default:
        return "default";
    }
  };

  const getTimeAgo = (dateString: string) => {
    const date = new Date(dateString);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMins = Math.floor(diffMs / 60000);
    const diffHours = Math.floor(diffMins / 60);
    const diffDays = Math.floor(diffHours / 24);

    if (!Number.isFinite(date.getTime())) return "Recently";
    if (diffDays > 0) return `${diffDays}d ago`;
    if (diffHours > 0) return `${diffHours}h ago`;
    if (diffMins > 0) return `${diffMins}m ago`;
    return "Just now";
  };

  const getItemTimeLabel = (item: DigestItem) =>
    item.source.toLowerCase() === "github" ? "Trending today" : getTimeAgo(item.published_at);

  const getHost = (url: string) => {
    try {
      return new URL(url).hostname.replace(/^www\./, "");
    } catch {
      return "";
    }
  };

  const toggleExpand = (id: string) => {
    setExpandedId((previous) => (previous === id ? null : id));
  };

  const openLink = async (event: React.MouseEvent, url: string) => {
    event.stopPropagation();
    await openUrl(url);
  };

  const toggleStarred = async (event: React.MouseEvent, id: string) => {
    event.stopPropagation();
    try {
      const starred: number = await invoke("toggle_starred", { id });
      setItems((previous) => {
        if (activeView === "saved" && starred === 0) {
          return previous.filter((item) => item.id !== id);
        }
        return previous.map((item) => item.id === id ? { ...item, starred } : item);
      });
    } catch (toggleError: any) {
      setError(toggleError.toString());
    }
  };

  const toggleSource = async (id: string) => {
    try {
      const updated: Source = await invoke("toggle_source", { id });
      setSources((previous) => previous.map((source) => source.id === id ? updated : source));
    } catch (toggleError: any) {
      setError(toggleError.toString());
    }
  };

  const renderSourceBadge = (item: DigestItem, compact = false) => {
    const style = getSourceStyle(item.source);
    return (
      <span className={`source-badge source-badge-${getSourceClass(item.source)} ${compact ? "source-badge-compact" : ""}`}>
        <span className="material-symbols-outlined source-badge-icon">{style.icon}</span>
        {item.source}
      </span>
    );
  };

  const renderThumbnail = (item: DigestItem, variant: "feature" | "story" | "detail") => {
    const style = getSourceStyle(item.source);
    const hasThumbnail = Boolean(item.thumbnail_url && !failedImageIds.has(item.id));

    return (
      <div className={`story-image story-image-${variant} ${hasThumbnail ? "" : "story-image-fallback"}`}>
        {hasThumbnail ? (
          <img
            src={item.thumbnail_url!}
            alt=""
            loading={variant === "detail" ? undefined : "lazy"}
            className="story-image-real"
            onError={() => handleImageError(item.id)}
          />
        ) : (
          <div
            className="story-image-placeholder"
            style={{ background: `linear-gradient(135deg, ${style.color}88, #0e0e0e 80%)` }}
          >
            <span className="material-symbols-outlined story-image-icon">{style.icon}</span>
            <span className="story-image-source">{item.source}</span>
          </div>
        )}
      </div>
    );
  };

  const renderBookmark = (item: DigestItem) => (
    <button
      type="button"
      aria-label={item.starred ? "Saved" : "Save"}
      className={`card-action ${item.starred ? "is-starred" : ""}`}
      onClick={(event) => void toggleStarred(event, item.id)}
    >
      <span className="material-symbols-outlined">{item.starred ? "bookmark" : "bookmark_border"}</span>
    </button>
  );

  const renderFeaturedCard = (item: DigestItem) => (
    <article className="stream-card featured-card" onClick={() => toggleExpand(item.id)}>
      <div className="featured-image-wrap">
        {renderThumbnail(item, "feature")}
        <div className="featured-badges">
          {renderSourceBadge(item, true)}
          <span className="trend-badge">{item.source.toLowerCase() === "github" ? "Trending" : "Top story"}</span>
        </div>
        <div className="featured-bookmark">{renderBookmark(item)}</div>
      </div>
      <div className="featured-body">
        <div className="story-kicker">
          <span className="story-source-name">{item.source}</span>
          <span className="story-time">{getItemTimeLabel(item)}</span>
        </div>
        <h2>{item.title}</h2>
        {item.summary && <p>{item.summary}</p>}
        <div className="card-metadata">
          <span className="metadata-category">{item.category || "Developer news"}</span>
          <span>{getHost(item.url) || item.source}</span>
        </div>
      </div>
    </article>
  );

  const renderStreamCard = (item: DigestItem) => {
    const hasThumbnail = Boolean(item.thumbnail_url && !failedImageIds.has(item.id));

    return (
      <article
        className={`stream-card standard-card ${hasThumbnail ? "standard-card-with-image" : "standard-card-compact"}`}
        onClick={() => toggleExpand(item.id)}
      >
        {hasThumbnail && (
          <div className="standard-image-wrap">
            {renderThumbnail(item, "story")}
            <div className="standard-bookmark">{renderBookmark(item)}</div>
          </div>
        )}
        <div className="standard-body">
          <div className="standard-topline">
            {renderSourceBadge(item, true)}
            {!hasThumbnail && renderBookmark(item)}
          </div>
          <h2>{item.title}</h2>
          {item.summary && <p>{item.summary}</p>}
          {getHost(item.url) && <div className="story-link">{getHost(item.url)}</div>}
          <div className="standard-footer">
            <span>{item.category || item.source}</span>
            <span>{getItemTimeLabel(item)}</span>
          </div>
        </div>
      </article>
    );
  };

  const renderSkeletonCard = (className: string) => (
    <div className={`skeleton-card ${className}`}>
      <div className="skeleton-block skeleton-image" />
      <div className="skeleton-copy">
        <div className="skeleton-line skeleton-line-short" />
        <div className="skeleton-line" />
        <div className="skeleton-line skeleton-line-medium" />
      </div>
    </div>
  );

  const renderFeed = () => {
    if (loading && items.length === 0) {
      return (
        <div className="stream-grid skeleton-grid" aria-label="Loading feed">
          <div className="stream-column">{renderSkeletonCard("skeleton-feature")}{renderSkeletonCard("skeleton-compact")}</div>
          <div className="stream-column">{renderSkeletonCard("skeleton-story")}{renderSkeletonCard("skeleton-compact")}</div>
          <div className="stream-column">{renderSkeletonCard("skeleton-compact")}{renderSkeletonCard("skeleton-story")}</div>
        </div>
      );
    }

    if (items.length === 0) {
      return (
        <div className="empty-state">
          <span className="material-symbols-outlined">inbox</span>
          <p>{activeView === "saved" ? "No saved stories yet." : "No news found for this stream."}</p>
        </div>
      );
    }

    const featured = items[0];
    const remaining = items.slice(1);
    const columns = [
      remaining.filter((_, index) => index % 3 === 0),
      remaining.filter((_, index) => index % 3 === 1),
      remaining.filter((_, index) => index % 3 === 2),
    ];

    return (
      <div className="stream-grid">
        <div className="stream-column">
          {featured && renderFeaturedCard(featured)}
          {columns[0].map((item) => <React.Fragment key={item.id}>{renderStreamCard(item)}</React.Fragment>)}
        </div>
        <div className="stream-column">
          {columns[1].map((item) => <React.Fragment key={item.id}>{renderStreamCard(item)}</React.Fragment>)}
        </div>
        <div className="stream-column">
          {columns[2].map((item) => <React.Fragment key={item.id}>{renderStreamCard(item)}</React.Fragment>)}
        </div>
      </div>
    );
  };

  const getSourceIcon = (id: string) => {
    switch (id) {
      case "github":
        return "code";
      case "hackernews":
        return "forum";
      case "arxiv":
        return "science";
      default:
        return "rss_feed";
    }
  };

  const renderSourcesView = () => {
    if (sourcesLoading && sources.length === 0) {
      return (
        <div className="sources-panel sources-loading">
          <span className="material-symbols-outlined spin">sync</span>
          <p>Loading local sources...</p>
        </div>
      );
    }

    return (
      <section className="sources-panel" id="sources">
        <div className="sources-panel-heading">
          <div>
            <p className="eyebrow">LOCAL CONFIGURATION</p>
            <h2>Sources</h2>
            <p>Choose which local feeds are allowed to refresh your digest.</p>
          </div>
          <span className="local-only-badge">
            <span className="material-symbols-outlined">database</span>
            Stored locally
          </span>
        </div>
        <div className="source-settings-list">
          {sources.map((source) => (
            <div className="source-setting-row" key={source.id}>
              <div className={`source-setting-icon source-setting-${source.id}`}>
                <span className="material-symbols-outlined">{getSourceIcon(source.id)}</span>
              </div>
              <div className="source-setting-copy">
                <h3>{source.name}</h3>
                <p>{source.enabled ? "Included in the next refresh" : "Skipped on the next refresh"}</p>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={source.enabled === 1}
                aria-label={`Toggle ${source.name}`}
                className={`source-switch ${source.enabled ? "enabled" : ""}`}
                onClick={() => void toggleSource(source.id)}
              >
                <span />
              </button>
            </div>
          ))}
        </div>
      </section>
    );
  };

  return (
    <div className="app-shell">
      <header className="top-nav">
        <div className="top-nav-inner">
          <a className="brand-mark" href="#top" aria-label="Bytewhir home">Bytewhir</a>

          <div className="search-shell" aria-label="Search developer news">
            <span className="material-symbols-outlined">search</span>
            <input readOnly placeholder="Search developer news..." aria-label="Search developer news" />
          </div>

          <div className="top-actions">
            <nav className="desktop-links" aria-label="Primary navigation">
              <button type="button" className={activeView === "feed" ? "active" : ""} onClick={() => setActiveView("feed")}>Feed</button>
              <button type="button" className={activeView === "sources" ? "active" : ""} onClick={() => setActiveView("sources")}>Sources</button>
              <button type="button" className={activeView === "saved" ? "active" : ""} onClick={() => setActiveView("saved")}>Saved</button>
            </nav>
            <button
              type="button"
              aria-label="Refresh feed"
              className="refresh-button"
              onClick={fetchNetworkDigest}
              disabled={loading}
            >
              <span className={`material-symbols-outlined ${loading ? "spin" : ""}`}>refresh</span>
            </button>
            <div className="profile-orb" aria-label="Bytewhir profile">B</div>
          </div>
        </div>
      </header>

      <main className="feed-page" id="top">
        <section className="stream-heading" id="feed">
          <div>
            <p className="eyebrow">BYTEWHIR / DEVELOPER NEWS</p>
            <h1>{activeView === "saved" ? "Saved Stories" : activeView === "sources" ? "Source Settings" : "Curated Stream"}</h1>
            <p>{activeView === "saved" ? "Your locally bookmarked developer news." : activeView === "sources" ? "Control which feeds refresh your local digest." : "The latest in tech, open-source, and AI."}</p>
          </div>
          {activeView === "feed" && (
            <div className="filter-row" role="tablist" aria-label="News streams">
              {FILTERS.map((filter) => (
                <button
                  type="button"
                  role="tab"
                  aria-selected={activeFilter === filter.value}
                  key={filter.value}
                  onClick={() => setActiveFilter(filter.value)}
                  className={`filter-pill ${activeFilter === filter.value ? "active" : ""}`}
                >
                  {filter.label}
                </button>
              ))}
            </div>
          )}
        </section>

        {error && <div className="error-banner">{error}</div>}

        {activeView === "sources" ? renderSourcesView() : renderFeed()}

        {activeView !== "sources" && items.length > 0 && (
          <button
            type="button"
            onClick={() => loadFeed(true)}
            disabled={loadingOlder}
            className="load-more-button"
          >
            {loadingOlder ? <span className="material-symbols-outlined spin">sync</span> : "Load older stories"}
          </button>
        )}
      </main>

      <footer className="site-footer">
        <span className="footer-brand">Bytewhir</span>
        <div className="footer-links">
          <a href="#privacy">Privacy Policy</a>
          <a href="#terms">Terms of Service</a>
          <a href="#contact">Contact</a>
        </div>
        <span>© 2024 Bytewhir Developer News</span>
      </footer>

      <button
        type="button"
        aria-label="Manual Refresh"
        onClick={fetchNetworkDigest}
        className="mobile-refresh"
        disabled={loading}
      >
        <span className={`material-symbols-outlined ${loading ? "spin" : ""}`}>sync</span>
      </button>

      <nav className="mobile-bottom-nav" aria-label="Mobile navigation">
        <button type="button" className={activeView === "feed" ? "active" : ""} onClick={() => setActiveView("feed")}>
          <span className="material-symbols-outlined">dynamic_feed</span>
          Feed
        </button>
        <button type="button" className={activeView === "sources" ? "active" : ""} onClick={() => setActiveView("sources")}>
          <span className="material-symbols-outlined">hub</span>
          Sources
        </button>
        <button type="button" className={activeView === "saved" ? "active" : ""} onClick={() => setActiveView("saved")}>
          <span className="material-symbols-outlined">bookmark</span>
          Saved
        </button>
      </nav>

      {expandedId && items.find((item) => item.id === expandedId) && (() => {
        const item = items.find((candidate) => candidate.id === expandedId)!;
        const style = getSourceStyle(item.source);
        const hasThumbnail = Boolean(item.thumbnail_url && !failedImageIds.has(item.id));

        return (
          <div className="detail-overlay">
            <div className="detail-toolbar">
              <button type="button" aria-label="Go back" onClick={() => setExpandedId(null)} className="detail-icon-button">
                <span className="material-symbols-outlined">arrow_back</span>
              </button>
              <div className="detail-actions">
                <button
                  type="button"
                  aria-label={item.starred ? "Remove from Saved" : "Save"}
                  className={`detail-icon-button ${item.starred ? "is-starred" : ""}`}
                  onClick={(event) => void toggleStarred(event, item.id)}
                >
                  <span className="material-symbols-outlined">{item.starred ? "bookmark" : "bookmark_border"}</span>
                </button>
                <button type="button" aria-label="Share" className="detail-icon-button">
                  <span className="material-symbols-outlined">share</span>
                </button>
              </div>
            </div>

            <article className="detail-card">
              <div className="detail-image">
                {hasThumbnail ? (
                  <img src={item.thumbnail_url!} alt="" onError={() => handleImageError(item.id)} />
                ) : (
                  <div
                    className="story-image-placeholder"
                    style={{ background: `linear-gradient(135deg, ${style.color}88, #0e0e0e 80%)` }}
                  >
                    <span className="material-symbols-outlined story-image-icon">{style.icon}</span>
                    <span className="story-image-source">{item.source}</span>
                  </div>
                )}
                <div className="detail-image-gradient" />
                <div className="detail-image-meta">
                  {renderSourceBadge(item, true)}
                  <span>{getItemTimeLabel(item)}</span>
                </div>
              </div>
              <div className="detail-content">
                {item.category && <span className="detail-category">{item.category}</span>}
                <h1>{item.title}</h1>
                <p>{item.summary || "No summary available."}</p>
              </div>
              <div className="detail-footer">
                <span>{getHost(item.url) || "ARTICLE"}</span>
                <button type="button" onClick={(event) => openLink(event, item.url)} className="read-button">
                  Read Full Article
                  <span className="material-symbols-outlined">open_in_new</span>
                </button>
              </div>
            </article>
          </div>
        );
      })()}
    </div>
  );
}

function WrappedApp() {
  return (
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  );
}

export default WrappedApp;
