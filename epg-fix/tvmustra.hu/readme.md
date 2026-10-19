# tvmustra.hu

https://www.tvmustra.hu/

### Download the guide

```sh
npm run grab --- --sites=tvmustra.hu --maxConnections=1 --delay=1000
```
**IMPORTANT:** Keep in mind that the new page only accepts 1 connection `--maxConnections=1` at a time (Cloudflare protection). In this case, it works only with `--delay` (1000 = 1 second)

### Update channel list

```sh
npm run channels:parse --- --config=./sites/tvmustra.hu/tvmustra.hu.config.js --output=./sites/tvmustra.hu/tvmustra.hu.channels.xml
```

### Test

```sh
npm test --- tvmustra.hu
```
