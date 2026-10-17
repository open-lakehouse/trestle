# Authelia: sign-in for the gateway

This environment has sign-in turned on (`[selection.gateway] auth = true`). Authelia is the
gateway's forward-auth provider: every API and UI route on the shared listener asks it whether
the caller is signed in. Browsers get its login page; API clients send basic auth.

## Users

Accounts live in [`users.yml`](users.yml), in this directory
(`modules/authelia/users.yml`). The file starts with one demo account: username `demo`,
password `authelia`. Change or remove it before you share the environment.

To add, change, or remove users:

1. Edit `users.yml`. Each user is one block under `users:`. Copy the `demo` block and change
   the name, `displayname`, `email`, `groups`, and `password`.
2. Hash each password (Authelia stores argon2id hashes, not plain text):

   ```bash
   docker run --rm authelia/authelia:latest \
     authelia crypto hash generate argon2 --password 'choose-a-password'
   ```

   Paste the printed `$argon2id$…` digest as the user's `password`.
3. Restart Authelia from the environment's root so it rereads the file:

   ```bash
   docker compose restart authelia
   ```

A user's `email` and `displayname` reach the services behind the gateway as the
`Remote-Email` and `Remote-Name` headers, and `groups` as `Remote-Groups`. With policy on,
the default policy lets any signed-in user read and the `writers` or `admins` group write.

`trestle env render` writes `users.yml` only when it doesn't exist yet, so your edits survive
re-renders. To start over from the demo account, delete the file and render again.

## The other files here

Re-rendering regenerates them, so make changes through `trestle env` rather than by editing:

- `compose.yaml`: the Authelia service.
- `configuration.yml`: Authelia's configuration (session cookie domains, storage, access
  control).
- `.env/` and `secrets/`: generated settings and secrets, git-ignored.
