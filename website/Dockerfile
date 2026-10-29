# Static site for encore.nayhap.com. Build from this folder:
#   docker build -t encore-website website
#   docker run --rm -p 8080:80 encore-website
FROM nginx:1.27-alpine
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY index.html 404.html robots.txt sitemap.xml site.webmanifest favicon.ico /usr/share/nginx/html/
COPY assets /usr/share/nginx/html/assets
EXPOSE 80
HEALTHCHECK --interval=30s --timeout=3s CMD wget -qO- http://127.0.0.1/ >/dev/null || exit 1
