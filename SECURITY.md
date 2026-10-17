# Security

Report vulnerabilities through GitHub private vulnerability reporting when it is
enabled on the published repository. If unavailable, contact the maintainer
privately; do not put credentials or private reproduction data in public issues.
Include the Retriever version, platform, impact and a synthetic reproduction.

A coordinator pairing token grants trusted-operator access to its library and
file operations. Retriever is not a multi-tenant authorization boundary. Protect
the state directory and use loopback, SSH tunnels or restricted HTTPS for remote
access. Metadata owner labels do not establish OS access controls.

Native AI models run locally. Searchable text and model caches remain in the
selected coordinator state directory. Choosing folders for indexing makes their
eligible contents available to trusted paired clients. Secret screening cannot
identify every sensitive document; choose source folders deliberately.

The updater contacts the configured public GitHub repository without library
credentials or data. It validates versions, release identity, asset host, size
and SHA-256 before installation. Local shutdown requires a separate local secret
and refuses active work. The repository maintainer is the update trust authority;
protect release permissions, review CI changes and use OS code signing.

Automatic checks can be disabled in Settings. Updates never change filesystem
permissions to bypass OS trust or elevation prompts. Preview packages without
publisher signing are labeled accordingly. Disk Health remains fixture-only.
