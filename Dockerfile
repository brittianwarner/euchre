# syntax=docker/dockerfile:1
FROM oven/bun:1.4.2 AS build
WORKDIR /app
COPY package.json bun.lock ./
COPY rivetkit-svelte/package.json ./rivetkit-svelte/package.json
RUN bun install --frozen-lockfile --ignore-scripts
COPY . .
RUN bun run build

FROM oven/bun:1.4.2 AS dependencies
WORKDIR /app
COPY package.json bun.lock ./
COPY rivetkit-svelte/package.json ./rivetkit-svelte/package.json
RUN bun install --production --frozen-lockfile --ignore-scripts

FROM oven/bun:1.4.2
WORKDIR /app
ENV NODE_ENV=production PORT=3000
COPY --from=build /app/package.json /app/bun.lock ./
COPY --from=dependencies /app/node_modules ./node_modules
COPY --from=build /app/rivetkit-svelte ./rivetkit-svelte
COPY --from=build /app/build ./build
COPY --from=build /app/src ./src
COPY --from=build /app/server ./server
USER bun
EXPOSE 3000
CMD ["bun", "server/index.ts"]
