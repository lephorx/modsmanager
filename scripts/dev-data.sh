#!/usr/bin/env bash
# Mock data for `pnpm dev` (the "Mods Manager (dev)" app, its own data folder).
#
# - a local Postgres in Docker (container mm-dev-db, port 5433) instead of the shared database
# - a copy of the real games and mod lists (read-only from the installed app's database),
#   so downloads, updates and the Nexus window work with real files
# - pretend friends: presence (online, behind, synced) and an activity history
# - empty test game/content folders, so installing never touches the real game folders
#
# Usage: scripts/dev-data.sh          (again any time to reset the dev data)
#        scripts/dev-data.sh --empty  (no copied lists: just the presets' games, no mods)
set -euo pipefail

PORT=5433
NAME=mm-dev-db
IMAGE=postgres:18
DEV_URL="postgres://postgres:dev@localhost:$PORT/mods"
REAL_DIR="$APPDATA/dev.lephor.modsmanager"
DEV_DIR="$APPDATA/dev.lephor.modsmanager.dev"
MOCK_DIR="$LOCALAPPDATA/dev.lephor.modsmanager.dev/mock"

docker info >/dev/null 2>&1 || { echo "Docker isn't running; start Docker Desktop first" >&2; exit 1; }

if ! docker ps -a --format '{{.Names}}' | grep -qx "$NAME"; then
  docker run -d --name "$NAME" -e POSTGRES_PASSWORD=dev -e POSTGRES_DB=mods -p "$PORT:5432" "$IMAGE" >/dev/null
fi
docker start "$NAME" >/dev/null
until docker exec "$NAME" pg_isready -U postgres -d mods >/dev/null 2>&1; do sleep 1; done
# pg_isready answers before the init scripts finish; wait for a real query
until docker exec "$NAME" psql -U postgres -d mods -c 'select 1' >/dev/null 2>&1; do sleep 1; done

psql() { docker exec -i "$NAME" psql -U postgres -d mods -v ON_ERROR_STOP=1 -q "$@"; }

echo "Resetting the dev database"
psql <<'SQL'
DROP TABLE IF EXISTS mm_games, mm_mods, mm_events, mm_presence, mm_members CASCADE;
CREATE TABLE mm_games (
    id BIGSERIAL PRIMARY KEY, name TEXT NOT NULL, modio_id BIGINT NOT NULL DEFAULT 0,
    modio_slug TEXT NOT NULL DEFAULT '', thunderstore TEXT NOT NULL DEFAULT '', loader TEXT NOT NULL DEFAULT 'none',
    steam_dir TEXT NOT NULL DEFAULT '', content_hint TEXT NOT NULL DEFAULT '', created_by TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), nexus TEXT NOT NULL DEFAULT '');
CREATE TABLE mm_mods (
    id BIGSERIAL PRIMARY KEY, source TEXT NOT NULL, source_id TEXT NOT NULL, target TEXT NOT NULL DEFAULT 'content',
    name TEXT NOT NULL, version TEXT NOT NULL DEFAULT '', file_ref TEXT NOT NULL DEFAULT '', file_hash TEXT NOT NULL DEFAULT '',
    file_size BIGINT NOT NULL DEFAULT 0, page_url TEXT NOT NULL DEFAULT '', thumb_url TEXT NOT NULL DEFAULT '',
    summary TEXT NOT NULL DEFAULT '', author TEXT NOT NULL DEFAULT '', extra JSONB NOT NULL DEFAULT '{}',
    added_by TEXT NOT NULL, added_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_by TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(), game_id BIGINT);
SQL

REAL_URL=""
if [ "${1:-}" != "--empty" ] && [ -f "$REAL_DIR/settings.json" ]; then
  REAL_URL=$(python -I -c "import json,sys; print(json.load(open(sys.argv[1], encoding='utf-8')).get('databaseUrl',''))" "$REAL_DIR/settings.json")
fi

if [ -n "$REAL_URL" ]; then
  echo "Copying the real games and mod lists (read only)"
  # COPY ... TO STDOUT only reads; the real database isn't changed
  for t in mm_games mm_mods; do
    cols=$([ $t = mm_games ] && echo "id,name,modio_id,modio_slug,thunderstore,loader,steam_dir,content_hint,created_by,created_at,nexus" \
      || echo "id,source,source_id,target,name,version,file_ref,file_hash,file_size,page_url,thumb_url,summary,author,extra,added_by,added_at,updated_by,updated_at,game_id")
    docker exec "$NAME" psql "$REAL_URL" -v ON_ERROR_STOP=1 -q -c "\\copy (SELECT $cols FROM $t) TO STDOUT" \
      | psql -c "\\copy $t ($cols) FROM STDIN"
  done
else
  echo "Adding the preset games (no mods)"
  psql <<'SQL'
INSERT INTO mm_games (name, modio_id, modio_slug, thunderstore, loader, steam_dir, content_hint, created_by, nexus) VALUES
 ('BONELAB', 3809, 'bonelab', 'bonelab', 'melonloader', 'BONELAB', '%LOCALLOW%/Stress Level Zero/BONELAB/Mods', 'lephor', 'bonelab'),
 ('Blade & Sorcery', 0, 'bladeandsorcery', '', 'none', 'Blade & Sorcery', '%GAME%/BladeAndSorcery_Data/StreamingAssets/Mods', 'lephor', 'bladeandsorcery'),
 ('Lethal Company', 0, '', 'lethal-company', 'bepinex', 'Lethal Company', '', 'lephor', 'lethalcompany');
SQL
fi

echo "Adding pretend friends and activity"
psql <<'SQL'
SELECT setval('mm_games_id_seq', GREATEST((SELECT max(id) FROM mm_games), 1));
SELECT setval('mm_mods_id_seq', GREATEST((SELECT max(id) FROM mm_mods), 1));
-- the app creates the rest of the schema (mm_events, mm_presence, indexes) the same way it does
-- for any database; create them here too so the mock rows can go in now
CREATE TABLE mm_events (id BIGSERIAL PRIMARY KEY, at TIMESTAMPTZ NOT NULL DEFAULT now(), actor TEXT NOT NULL,
    action TEXT NOT NULL, mod_name TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '', game_id BIGINT);
CREATE TABLE mm_presence (name TEXT NOT NULL, game_id BIGINT NOT NULL, last_seen TIMESTAMPTZ NOT NULL DEFAULT now(),
    pending INT NOT NULL DEFAULT 0, synced_at TIMESTAMPTZ, PRIMARY KEY (name, game_id));
CREATE UNIQUE INDEX mm_mods_game_source ON mm_mods (game_id, source, source_id);
CREATE INDEX mm_events_game ON mm_events (game_id, id DESC);

-- Alex: online and up to date · Sam: online, 3 behind · Jordan: seen yesterday · Riley: seen last week
INSERT INTO mm_presence (name, game_id, last_seen, pending, synced_at)
SELECT p.name, g.id, now() - p.ago, p.pending, now() - p.ago - interval '5 minutes'
FROM mm_games g, (VALUES ('Alex', interval '20 seconds', 0), ('Sam', interval '40 seconds', 3),
                         ('Jordan', interval '26 hours', 1), ('Riley', interval '8 days', 0)) AS p(name, ago, pending);

-- history: who added and updated what, spread over the last two weeks
INSERT INTO mm_events (at, actor, action, mod_name, detail, game_id)
SELECT m.added_at - (row_number() OVER (PARTITION BY m.game_id ORDER BY m.id)) * interval '7 hours',
       (ARRAY['Alex','Sam','Jordan','Riley','lephor'])[1 + (m.id % 5)], 'add', m.name, m.version, m.game_id
FROM mm_mods m;
INSERT INTO mm_events (at, actor, action, mod_name, detail, game_id)
SELECT now() - (m.id % 9) * interval '13 hours', (ARRAY['Sam','Alex','Jordan'])[1 + (m.id % 3)], 'update', m.name, m.version, m.game_id
FROM mm_mods m WHERE m.id % 3 = 0;
INSERT INTO mm_events (at, actor, action, mod_name, detail, game_id)
SELECT now() - interval '3 days', 'Riley', 'remove', 'Old Weapon Pack', '', g.id FROM mm_games g;
SQL

echo "Writing the dev app's settings (test folders, local database)"
rm -rf "$MOCK_DIR"
mkdir -p "$DEV_DIR"
ids=$(psql -At -c "SELECT id || ':' || name FROM mm_games ORDER BY id")
python -I - "$REAL_DIR/settings.json" "$DEV_DIR/settings.json" "$DEV_URL" "$MOCK_DIR" "$ids" <<'PY'
import json, os, re, sys
real, out, url, mock, ids = sys.argv[1:6]
try:
    s = json.load(open(real, encoding="utf-8"))
except OSError:
    s = {}
games = {}
for line in filter(None, ids.splitlines()):
    gid, name = line.split(":", 1)
    slug = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    game, content = os.path.join(mock, slug, "game"), os.path.join(mock, slug, "content")
    os.makedirs(game, exist_ok=True)
    os.makedirs(content, exist_ok=True)
    games[gid] = {"contentDir": content, "gameDir": game}
s.update({
    "name": s.get("name") or "lephor",
    "databaseUrl": url,
    "games": games,
    "currentGame": s.get("currentGame") if str(s.get("currentGame")) in games else (int(next(iter(games))) if games else None),
    "autoSync": False,
})
json.dump(s, open(out, "w", encoding="utf-8"), indent=2)
PY
# the copied local.json described the real folders; the test folders start empty
rm -f "$DEV_DIR/local.json"

echo
echo "Done. Dev database: $DEV_URL"
echo "Test folders: $MOCK_DIR"
psql -c "SELECT g.id, g.name, count(m.id) AS mods, count(m.id) FILTER (WHERE m.source = 'nexus') AS nexus FROM mm_games g LEFT JOIN mm_mods m ON m.game_id = g.id GROUP BY g.id ORDER BY g.id"
