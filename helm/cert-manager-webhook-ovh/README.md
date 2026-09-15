# cert-manager-webhook-ovh — runbook

Enables DNS-01 challenges against OVH-hosted DNS so cert-manager can issue a
wildcard certificate for `*.bunker.skolln.com`. `skolln.com`'s nameservers
are OVH's (`ns102.ovh.net` / `dns102.ovh.net`), confirmed via `dig NS
skolln.com`.

This is a **one-time, cluster-wide** setup — once done, every current and
future Bunker organization subdomain gets HTTPS automatically, no per-org
manual step.

## 1. Create an OVH API application (you do this — needs your OVH account)

Open this link (already scoped to exactly what the webhook needs — read/write
on your DNS zones, nothing else):

```
https://api.ovh.com/createToken/?GET=/domain/zone/*&PUT=/domain/zone/*&POST=/domain/zone/*&DELETE=/domain/zone/*
```

Fill in an application name (e.g. "bunker-cert-manager-dns01") and a
description, submit, and OVH gives you three values: **Application Key**,
**Application Secret**, **Consumer Key**. Keep them somewhere safe for the
next step — don't paste them into a chat or commit them anywhere.

## 2. Create the Kubernetes Secret (you run this — never commit real values)

```bash
kubectl create secret generic ovh-dns01-credentials \
  --namespace cert-manager \
  --from-literal=applicationKey='<your Application Key>' \
  --from-literal=applicationSecret='<your Application Secret>' \
  --from-literal=applicationConsumerKey='<your Consumer Key>'
```

## 3. Install the webhook

```bash
helm repo add cert-manager-webhook-ovh-charts https://aureq.github.io/cert-manager-webhook-ovh/
helm repo update
helm upgrade --install --namespace cert-manager \
  -f helm/cert-manager-webhook-ovh/values.yaml \
  cm-webhook-ovh cert-manager-webhook-ovh-charts/cert-manager-webhook-ovh
```

This also creates the `bunker-dns01-issuer` `ClusterIssuer` (defined in
`values.yaml` here, `issuers[0]`). Verify it comes up ready:

```bash
kubectl get clusterissuer bunker-dns01-issuer
```

## 4. Point Bunker's Ingress at the wildcard issuer

Once the `ClusterIssuer` is `READY: True`, deploy the Bunker chart change
that switches to the wildcard host and this new issuer (see
`helm/bunker/values.yaml` — `ingress.clusterIssuer: bunker-dns01-issuer`,
`ingress.wildcardHost: "*.bunker.skolln.com"`):

```bash
helm upgrade --install bunker ./helm/bunker --namespace bunker --create-namespace --set image.tag=<current tag> --wait
```

Then confirm the new certificate is issued and covers both names:

```bash
kubectl get certificate -n bunker
echo | openssl s_client -connect alume.bunker.skolln.com:443 -servername alume.bunker.skolln.com 2>/dev/null | openssl x509 -noout -issuer -ext subjectAltName
```

The issuer should now be Let's Encrypt, not `TRAEFIK DEFAULT CERT`, and the
SAN list should include `*.bunker.skolln.com`.
