import { useEffect, useState } from "react";
import { api, type Card, type QueryResult } from "../api/client";
import { ResultView } from "./ResultView";

export function SavedQuestions() {
  const [cards, setCards] = useState<Card[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [result, setResult] = useState<QueryResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .cards()
      .then(setCards)
      .catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  async function run(card: Card) {
    setError(null);
    setActive(card.id);
    try {
      setResult(await api.runCard(card.id));
    } catch (e: unknown) {
      setResult(null);
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <div className="saved">
      <h2>Saved questions</h2>
      {cards.length === 0 ? (
        <p className="muted">No saved questions yet. Build one in Explore and save it.</p>
      ) : (
        <ul className="saved__list">
          {cards.map((c) => (
            <li key={c.id}>
              <button className="link" onClick={() => run(c)} data-active={c.id === active}>
                {c.name}
              </button>
              <span className="muted"> · {c.query.source_table}</span>
            </li>
          ))}
        </ul>
      )}
      {error && <p className="app__error">{error}</p>}
      {result && <ResultView result={result} />}
    </div>
  );
}
