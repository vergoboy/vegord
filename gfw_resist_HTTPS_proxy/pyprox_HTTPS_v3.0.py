#!/usr/bin/env python3

import dns.message
import dns.rdatatype
import requests
from pathlib import Path
import os
import base64
import socket
import threading
import time
import random
import json
from datetime import datetime


CONFIG = {
    'listen_PORT': 4500,
    'num_fragment': 12,
    'fragment_sleep': 0.002,
    'log_every_N_sec': 30,
    'allow_insecure': True,
    'my_socket_timeout': 60,
    'voice_socket_timeout': 120,
    'first_time_sleep': 0.02,
    'accept_time_sleep': 0.002,
    'doh_max_retries': 3,
    'doh_max_fails_before_switch': 2,
    'doh_blacklist_sec': 60,
    'doh_timeout': 5,
    'discord_ping_interval': 10,
    'discord_ping_timeout': 2,
    'discord_max_ips': 20,
}

DoH_servers = [
    'https://cloudflare-dns.com/dns-query?dns=',
    'https://dns.google/dns-query?dns=',
    'https://doh.opendns.com/dns-query?dns=',
    'https://dns.quad9.net/dns-query?dns=',
    'https://doh.libredns.gr/dns-query?dns=',
    'https://dns.bitdefender.net/dns-query?dns=',
    'https://secure.avastdns.com/dns-query?dns=',
    'https://doh.cleanbrowsing.org/doh/dns-query?dns=',
    'https://doh.dns.sb/doh/dns-query?dns=',
    'https://doh.tiar.app/dns-query?dns=',
    'https://doh.dnswarden.com/dns-query?dns=',
    'https://doh.powerdns.org/dns-query?dns=',
    'https://dns.electrotm.org/dns-query?dns=',
    'https://cluster-1.gac.edu/dns-query?dns=',
    'https://dns.hostux.net/dns-query?dns=',
    'https://doh.securedns.eu/dns-query?dns=',
    'https://doh.ffmuc.net/dns-query?dns=',
    'https://dns.cmrg.net/dns-query?dns=',
    'https://doh.centraleu.pi-dns.com/dns-query?dns=',
    'https://doh.dns.live/dns-query?dns=',
    'https://dns.friendi.ca/dns-query?dns=',
    'https://doh.bortzmeyer.org/dns-query?dns=',
    'https://doh.airdns.org/dns-query?dns=',
    'https://dns.hyperpipe.surge.sh/dns-query?dns=',
    'https://dns.digitale-gesellschaft.ch/dns-query?dns=',
    'https://doh.ibk.pl/dns-query?dns=',
    'https://dns.rubyfish.io/dns-query?dns=',
    'https://doh.otk.ee/dns-query?dns=',
    'https://dns.joatmalatesta.net/dns-query?dns=',
    'https://doh.shecan.ir/dns-query?dns=',
    'https://1.1.1.1/dns-query?dns=',
    'https://1.0.0.1/dns-query?dns=',
    'https://8.8.8.8/dns-query?dns=',
    'https://8.8.4.4/dns-query?dns=',
    'https://9.9.9.9/dns-query?dns=',
    'https://149.112.112.112/dns-query?dns=',
    'https://208.67.222.222/dns-query?dns=',
    'https://208.67.220.220/dns-query?dns=',
    'https://185.228.168.9/dns-query?dns=',
    'https://185.228.169.9/dns-query?dns=',
    'https://76.76.19.19/dns-query?dns=',
    'https://76.223.122.150/dns-query?dns=',
]

DNS_cache = {}
IP_DL_traffic = {}
IP_UL_traffic = {}
DoH_net_stats = {
    'total_queries': 0,
    'successful_queries': 0,
    'failed_queries': 0,
    'total_switch_count': 0,
    'current_doh_index': 0,
    'doh_perf': {i: {'rtt_ms': [], 'fail_count': 0, 'success_count': 0, 'last_rtt': None} for i in range(len(DoH_servers))},
    'start_time': None,
    'conn_total': 0,
    'conn_success': 0,
    'conn_filtered': 0,
}

offline_DNS = {
    'cloudflare-dns.com': '203.32.120.226',
    'dns.google': '8.8.8.8',
    'doh.opendns.com': '208.67.222.222',
    'dns.quad9.net': '9.9.9.9',
    'doh.libredns.gr': '116.202.176.26',
    'dns.bitdefender.net': '34.84.232.67',
    'secure.avastdns.com': '185.185.133.66',
    'doh.cleanbrowsing.org': '185.228.168.9',
    'doh.dns.sb': '185.184.71.198',
    'doh.tiar.app': '5.9.52.91',
    'doh.dnswarden.com': '116.203.249.54',
    'doh.powerdns.org': '188.166.104.87',
    'dns.electrotm.org': '78.157.42.100',
    'cluster-1.gac.edu': '138.236.128.101',
    'dns.hostux.net': '185.121.177.177',
    'doh.securedns.eu': '146.185.167.43',
    'doh.ffmuc.net': '5.1.66.255',
    'dns.cmrg.net': '199.58.81.218',
    'doh.centraleu.pi-dns.com': '116.202.120.165',
    'doh.dns.live': '104.28.1.1',
    'dns.friendi.ca': '198.50.200.234',
    'doh.bortzmeyer.org': '193.70.85.187',
    'doh.airdns.org': '37.120.215.68',
    'dns.hyperpipe.surge.sh': '188.114.97.3',
    'dns.digitale-gesellschaft.ch': '185.95.218.42',
    'doh.ibk.pl': '194.181.253.3',
    'dns.rubyfish.io': '139.162.235.169',
    'doh.otk.ee': '95.216.224.92',
    'dns.joatmalatesta.net': '192.161.48.7',
    'doh.shecan.ir': '178.22.122.100',
    'api.twitter.com': '104.244.42.66',
    'twitter.com': '104.244.42.1',
    'pbs.twimg.com': '93.184.220.70',
    'abs-0.twimg.com': '104.244.43.131',
    'abs.twimg.com': '152.199.24.185',
    'video.twimg.com': '192.229.220.133',
    't.co': '104.244.42.69',
    'ton.local.twitter.com': '104.244.42.1',
    'instagram.com': '163.70.128.174',
    'www.instagram.com': '163.70.128.174',
    'static.cdninstagram.com': '163.70.132.63',
    'scontent.cdninstagram.com': '163.70.132.63',
    'privacycenter.instagram.com': '163.70.128.174',
    'help.instagram.com': '163.70.128.174',
    'l.instagram.com': '163.70.128.174',
    'e1.whatsapp.net': '163.70.128.60',
    'e2.whatsapp.net': '163.70.128.60',
    'e3.whatsapp.net': '163.70.128.60',
    'e4.whatsapp.net': '163.70.128.60',
    'e5.whatsapp.net': '163.70.128.60',
    'e6.whatsapp.net': '163.70.128.60',
    'e7.whatsapp.net': '163.70.128.60',
    'e8.whatsapp.net': '163.70.128.60',
    'e9.whatsapp.net': '163.70.128.60',
    'e10.whatsapp.net': '163.70.128.60',
    'e11.whatsapp.net': '163.70.128.60',
    'e12.whatsapp.net': '163.70.128.60',
    'e13.whatsapp.net': '163.70.128.60',
    'e14.whatsapp.net': '163.70.128.60',
    'e15.whatsapp.net': '163.70.128.60',
    'e16.whatsapp.net': '163.70.128.60',
    'dit.whatsapp.net': '185.60.219.60',
    'g.whatsapp.net': '185.60.218.54',
    'wa.me': '185.60.219.60',
    'web.whatsapp.com': '31.13.83.51',
    'whatsapp.net': '31.13.83.51',
    'whatsapp.com': '31.13.83.51',
    'cdn.whatsapp.net': '31.13.83.51',
    'snr.whatsapp.net': '31.13.83.51',
    'static.xx.fbcdn.net': '31.13.75.13',
    'scontent-mct1-1.xx.fbcdn.net': '31.13.75.13',
    'video-mct1-1.xx.fbcdn.net': '31.13.75.13',
    'video.fevn1-2.fna.fbcdn.net': '185.48.241.146',
    'video.fevn1-4.fna.fbcdn.net': '185.48.243.145',
    'scontent.xx.fbcdn.net': '185.48.240.146',
    'scontent.fevn1-1.fna.fbcdn.net': '185.48.240.145',
    'scontent.fevn1-2.fna.fbcdn.net': '185.48.241.145',
    'scontent.fevn1-3.fna.fbcdn.net': '185.48.242.146',
    'scontent.fevn1-4.fna.fbcdn.net': '185.48.243.147',
    'connect.facebook.net': '31.13.84.51',
    'facebook.com': '31.13.65.49',
    'developers.facebook.com': '31.13.84.8',
    'about.meta.com': '163.70.128.13',
    'meta.com': '163.70.128.13',
    'ocsp.pki.goog': '172.217.16.195',
    'googleads.g.doubleclick.net': '45.157.177.108',
    'fonts.gstatic.com': '142.250.185.227',
    'rr2---sn-vh5ouxa-hju6.googlevideo.com': '213.202.6.141',
    'jnn-pa.googleapis.com': '45.157.177.108',
    'static.doubleclick.net': '202.61.195.218',
    'rr4---sn-hju7en7k.googlevideo.com': '74.125.167.74',
    'rr1---sn-hju7en7r.googlevideo.com': '74.125.167.87',
    'play.google.com': '142.250.184.238',
    'rr3---sn-vh5ouxa-hjuz.googlevideo.com': '134.0.218.206',
    'rr3---sn-hju7enel.googlevideo.com': '74.125.98.40',
    'download.visualstudio.microsoft.com': '68.232.34.200',
    'i.ytimg.com': '142.250.186.150',
    'rr2---sn-hju7enel.googlevideo.com': '74.125.98.39',
    'rr2---sn-hju7en7k.googlevideo.com': '74.125.167.72',
    'rr3---sn-4g5lznl6.googlevideo.com': '74.125.173.40',
    'rr1---sn-hju7enll.googlevideo.com': '74.125.98.6',
    'rr6---sn-hju7en7r.googlevideo.com': '74.125.167.92',
    'www.gstatic.com': '142.250.185.99',
    'apis.google.com': '172.217.23.110',
    'adservice.google.com': '202.61.195.218',
    'mail.google.com': '142.250.186.37',
    'accounts.google.com': '172.217.16.205',
    'lh3.googleusercontent.com': '193.26.157.66',
    'accounts.youtube.com': '172.217.16.206',
    'ssl.gstatic.com': '142.250.184.195',
    'fonts.gstatic.com': '172.217.23.99',
    'rr4---sn-hju7enll.googlevideo.com': '74.125.98.9',
    'rr2---sn-hju7enll.googlevideo.com': '74.125.98.7',
    'rr1---sn-hju7enel.googlevideo.com': '74.125.98.38',
    'rr5---sn-vh5ouxa-hjuz.googlevideo.com': '134.0.218.208',
    'i1.ytimg.com': '172.217.18.14',
    'plos.org': '162.159.135.42',
    'fonts.googleapis.com': '89.58.57.45',
    'genweb.plos.org': '104.26.1.141',
    'static.ads-twitter.com': '146.75.120.157',
    'www.google-analytics.com': '142.250.185.174',
    'rr1---sn-vh5ouxa-hju6.googlevideo.com': '213.202.6.140',
    'rr5---sn-vh5ouxa-hju6.googlevideo.com': '213.202.6.144',
    'rr5---sn-nv47zn7y.googlevideo.com': '173.194.15.74',
    'safebrowsing.googleapis.com': '202.61.195.218',
    'rr5---sn-vh5ouxa-hju6.googlevideo.com': '213.202.6.144',
    'rr1---sn-hju7en7r.googlevideo.com': '74.125.167.87',
    'rr4---sn-vh5ouxa-hju6.googlevideo.com': '213.202.6.143',
    'rr4---sn-hju7en7r.googlevideo.com': '74.125.167.90',
    'r1---sn-hju7enel.googlevideo.com': '74.125.98.38',
    'rr1---sn-nv47zn7r.googlevideo.com': '173.194.15.38',
    'rr2---sn-vh5ouxa-hjuz.googlevideo.com': '134.0.218.205',
    'rr4---sn-nv47zn7r.googlevideo.com': '173.194.15.41',
    'rr4---sn-hju7en7r.googlevideo.com': '74.125.167.90',
    'www.google.com': '142.250.186.36',
    'youtube.com': '216.239.38.120',
    'youtu.be': '216.239.38.120',
    'www.youtube.com': '216.239.38.120',
    'i.ytimg.com': '216.239.38.120',
    'yt3.ggpht.com': '142.250.186.36',
}

discord_domains = {
    'discord.com', 'discord.gg', 'discordapp.com', 'discordapp.net',
    'discord.media', 'discordstatus.com',
    'gateway.discord.gg', 'gateway-us-east1-b.discord.gg',
    'cdn.discordapp.com', 'cdn.discord.com',
    'media.discordapp.net', 'media.discord.com',
    'status.discord.com', 'api.discord.com',
}
discord_ips = {}
discord_best_ip = None
discord_best_rtt = None
discord_lock = threading.Lock()

BASE_DIR = Path(__file__).resolve().parent


def now_iso():
    return datetime.now().isoformat(timespec='seconds')


def log_console(tag, msg):
    t = now_iso()
    print(f'[{t}] [{tag}] {msg}')


class DNS_over_Fragment:
    def __init__(self):
        self.req = requests.session()
        self.proxy = {
            'https': f'http://127.0.0.1:{CONFIG["listen_PORT"]}'
        }
        self.lock = threading.Lock()
        self.doh_log_path = os.path.join(BASE_DIR, 'DoH_switch_log.txt')
        self.blacklist = {}

    def _get_current_url(self):
        return DoH_servers[DoH_net_stats['current_doh_index']]

    def _switch_doh(self):
        with self.lock:
            DoH_net_stats['total_switch_count'] += 1
            old_idx = DoH_net_stats['current_doh_index']
            old_url = DoH_servers[old_idx]

            tried = set()
            while len(tried) < len(DoH_servers):
                DoH_net_stats['current_doh_index'] = (
                    DoH_net_stats['current_doh_index'] + 1
                ) % len(DoH_servers)
                idx = DoH_net_stats['current_doh_index']
                tried.add(idx)
                if time.time() >= self.blacklist.get(idx, 0):
                    break

            new_url = DoH_servers[DoH_net_stats['current_doh_index']]
            event = {
                'time': now_iso(),
                'from': old_url,
                'to': new_url,
                'total_switches': DoH_net_stats['total_switch_count'],
            }
            try:
                with open(self.doh_log_path, 'a') as f:
                    f.write(json.dumps(event) + '\n')
            except Exception:
                pass
            log_console('DoH SWITCH', f'#{DoH_net_stats["total_switch_count"]} {old_url} -> {new_url}')
            return new_url

    def _blacklist_current(self):
        idx = DoH_net_stats['current_doh_index']
        self.blacklist[idx] = time.time() + CONFIG['doh_blacklist_sec']
        log_console('DoH BLACKLIST',
                    f'{DoH_servers[idx]} blacklisted for {CONFIG["doh_blacklist_sec"]}s')

    def query(self, server_name):
        ip = offline_DNS.get(server_name)
        if ip:
            log_console('DNS', f'offline {server_name} -> {ip}')
            return ip

        ip = DNS_cache.get(server_name)
        if ip:
            log_console('DNS', f'cached {server_name} -> {ip}')
            return ip

        DoH_net_stats['total_queries'] += 1
        is_discord = _is_discord_domain(server_name)
        log_console('DNS', f'resolving {server_name} via DoH{" [discord]" if is_discord else ""}')

        params = {'type': 'A', 'ct': 'application/dns-message'}
        fail_count = 0
        max_tries = min(len(DoH_servers), CONFIG['doh_max_retries'])

        for attempt in range(max_tries):
            url = self._get_current_url()
            idx = DoH_net_stats['current_doh_index']
            try:
                qmsg = dns.message.make_query(server_name, 'A')
                qwire = qmsg.to_wire()
                qb64 = base64.urlsafe_b64encode(qwire).decode('utf-8').replace('=', '')

                t0 = time.time()
                ans = self.req.get(
                    url + qb64,
                    params=params,
                    headers={'accept': 'application/dns-message'},
                    proxies=self.proxy,
                    verify=not CONFIG['allow_insecure'],
                    timeout=CONFIG['doh_timeout'],
                )
                rtt = round((time.time() - t0) * 1000, 1)

                DoH_net_stats['doh_perf'][idx]['last_rtt'] = rtt
                DoH_net_stats['doh_perf'][idx]['rtt_ms'].append(rtt)
                if len(DoH_net_stats['doh_perf'][idx]['rtt_ms']) > 50:
                    DoH_net_stats['doh_perf'][idx]['rtt_ms'].pop(0)

                if ans.status_code == 200 and ans.headers.get('content-type') == 'application/dns-message':
                    answer = dns.message.from_wire(ans.content)
                    resolved = None
                    all_ips = []
                    for rrset in answer.answer:
                        if rrset.rdtype == dns.rdatatype.A:
                            if resolved is None:
                                resolved = rrset[0].address
                            for rec in rrset:
                                all_ips.append(rec.address)
                    if resolved:
                        DNS_cache[server_name] = resolved
                        if is_discord and all_ips:
                            _feed_discord_ips(all_ips)
                        DoH_net_stats['successful_queries'] += 1
                        DoH_net_stats['doh_perf'][idx]['success_count'] += 1
                        log_console('DNS', f'{server_name} -> {resolved} (RTT={rtt}ms)')
                        return resolved
                    else:
                        log_console('DoH WARN', f'No A record from server #{idx} for {server_name}')
                else:
                    log_console('DoH WARN',
                                f'Server #{idx} returned {ans.status_code} for {server_name}')

            except requests.exceptions.Timeout:
                log_console('DoH TIMEOUT', f'Server #{idx} timeout ({CONFIG["doh_timeout"]}s)')
            except requests.exceptions.ConnectionError as e:
                log_console('DoH CONN_ERR', f'Server #{idx} connection failed')
            except Exception as e:
                log_console('DoH ERR', f'Server #{idx}: {repr(e)}')

            DoH_net_stats['doh_perf'][idx]['fail_count'] += 1
            fail_count += 1
            if fail_count >= CONFIG['doh_max_fails_before_switch']:
                self._blacklist_current()
                self._switch_doh()
                fail_count = 0

        DoH_net_stats['failed_queries'] += 1
        log_console('DNS FAIL', f'All DoH servers failed for {server_name}')
        return None


class ThreadedServer:
    def __init__(self, host, port):
        self.doh = DNS_over_Fragment()
        self.host = host
        self.port = port
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.sock.bind((self.host, self.port))

    def listen(self):
        self.sock.listen(128)
        while True:
            client, addr = self.sock.accept()
            client.settimeout(CONFIG['my_socket_timeout'])
            time.sleep(CONFIG['accept_time_sleep'])
            t = threading.Thread(target=self._proxy_loop, args=(client,), daemon=True)
            t.start()

    def _proxy_loop(self, client):
        backend = self._handshake(client)
        if not backend:
            client.close()
            return

        if isinstance(backend, str):
            ip = backend
            IP_UL_traffic.setdefault(ip, 0)
            IP_DL_traffic.setdefault(ip, 0)
            client.close()
            return

        host, port = getattr(client, '_target', ('?', 0))
        is_voice = _is_voice_port(port) or _is_discord_voice(host)

        ip = backend.getpeername()[0]
        IP_UL_traffic.setdefault(ip, 0)
        IP_DL_traffic.setdefault(ip, 0)

        first = True
        bytes_up = 0
        bytes_down = 0
        t_start = time.time()

        while True:
            try:
                data = client.recv(16384)
                if not data:
                    break

                if first:
                    first = False
                    time.sleep(CONFIG['first_time_sleep'])
                    t = threading.Thread(target=self._pipe,
                                         args=(backend, client, 'down'), daemon=True)
                    t.start()
                    _send_fragment(data, backend)
                    if is_voice:
                        log_console('VOICE', f'Relaying {host}:{port} ({ip})')
                else:
                    backend.sendall(data)
                bytes_up += len(data)
                IP_UL_traffic[ip] += len(data)
            except Exception:
                break

        elapsed = time.time() - t_start
        if is_voice or elapsed > 10:
            log_console('VOICE' if is_voice else 'CONNECT', f'Closed {host}:{port} ({ip}) after {elapsed:.1f}s UL={bytes_up} DL={bytes_down}')
        time.sleep(2)
        client.close()
        backend.close()

    def _pipe(self, src, dst, direction):
        ip = src.getpeername()[0]
        while True:
            try:
                data = src.recv(16384)
                if not data:
                    break
                dst.sendall(data)
                if direction == 'down':
                    IP_DL_traffic[ip] += len(data)
            except Exception:
                break
        time.sleep(2)
        src.close()
        dst.close()

    def _handshake(self, client):
        data = client.recv(16384)
        if not data:
            return None

        if data[0] == 5:
            return self._socks5(client, data)

        if data[:7] == b'CONNECT':
            host, port = data.split(b' ')[1].split(b':')
            host = host.decode()
            port = int(port)
            client._target = (host, port)
            log_console('CONNECT', f'{host}:{port}')
            sock = self._connect(host, port, client)
            if sock and not isinstance(sock, str):
                _safe_send(client, b'HTTP/1.1 200 Connection established\r\nProxy-agent: MyProxy/1.0\r\n\r\n')
            return sock

        if data[:3] == b'GET' or data[:4] in (b'POST', b'HEAD') or \
           data[:7] == b'OPTIONS' or data[:3] == b'PUT' or \
           data[:6] == b'DELETE' or data[:5] in (b'PATCH', b'TRACE'):
            method = data.split(b' ')[0].decode()
            url = data.split(b' ')[1].decode().replace('http://', 'https://')
            log_console('REDIRECT', f'{method} http -> https {url}')
            resp = f'HTTP/1.1 302 Found\r\nLocation: {url}\r\nProxy-agent: MyProxy/1.0\r\n\r\n'
            _safe_send(client, resp.encode())
            client.close()
            return None

        log_console('UNKNOWN', str(data[:10]))
        _safe_send(client, b'HTTP/1.1 400 Bad Request\r\nProxy-agent: MyProxy/1.0\r\n\r\n')
        client.close()
        return None

    def _socks5(self, client, data):
        if len(data) < 2 or 0 not in data[2:2 + data[1]]:
            _safe_send(client, b'\x05\xff')
            client.close()
            return None
        _safe_send(client, b'\x05\x00')
        try:
            req = client.recv(16384)
        except Exception:
            return None
        if len(req) < 4 or req[0] != 5 or req[1] != 1:
            _safe_send(client, b'\x05\x07\x00\x01\x00\x00\x00\x00\x00\x00')
            client.close()
            return None

        atype = req[3]
        try:
            if atype == 1:
                host = socket.inet_ntoa(req[4:8])
                port = int.from_bytes(req[8:10], 'big')
            elif atype == 3:
                dlen = req[4]
                host = req[5:5 + dlen].decode()
                port = int.from_bytes(req[5 + dlen:7 + dlen], 'big')
            elif atype == 4:
                host = socket.inet_ntop(socket.AF_INET6, req[4:20])
                port = int.from_bytes(req[20:22], 'big')
            else:
                _safe_send(client, b'\x05\x08\x00\x01\x00\x00\x00\x00\x00\x00')
                client.close()
                return None
        except Exception:
            return None

        client._target = (host, port)
        log_console('SOCKS5', f'{host}:{port}')
        sock = self._connect(host, port, client)
        if sock and not isinstance(sock, str):
            _safe_send(client, b'\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00')
        return sock

    def _connect(self, host, port, client):
        DoH_net_stats['conn_total'] += 1
        is_voice = _is_voice_port(port) or _is_discord_voice(host)
        try:
            sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
            timeout = CONFIG['voice_socket_timeout'] if is_voice else CONFIG['my_socket_timeout']
            sock.settimeout(timeout)
            sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)

            try:
                socket.inet_aton(host)
                ip = host
            except socket.error:
                if _is_discord_domain(host) or is_voice:
                    ip = _best_discord_ip()
                    if ip:
                        log_console('DISCORD', f'routing {host} via best-IP {ip}')
                    else:
                        ip = self.doh.query(host)
                else:
                    ip = self.doh.query(host)
                if not ip:
                    log_console('DNS FAIL', f'Could not resolve {host} for voice connection' if is_voice else f'Could not resolve {host}')
                    _safe_send(client, b'HTTP/1.1 502 Bad Gateway (DNS failed)\r\nProxy-agent: MyProxy/1.0\r\n\r\n')
                    client.close()
                    sock.close()
                    return None

            t0 = time.time()
            sock.connect((ip, port))
            elapsed = round((time.time() - t0) * 1000, 1)
            DoH_net_stats['conn_success'] += 1

            if is_voice:
                log_console('VOICE', f'Connected {host} ({ip}):{port} in {elapsed}ms (timeout={timeout}s)')
            return sock

        except socket.timeout:
            DoH_net_stats['conn_filtered'] += 1
            log_console('VOICE TIMEOUT' if is_voice else 'TIMEOUT', f'{host} ({ip}):{port} after {CONFIG["my_socket_timeout"]}s')
            _safe_send(client, b'HTTP/1.1 502 Bad Gateway (timeout)\r\nProxy-agent: MyProxy/1.0\r\n\r\n')
            client.close()
            try:
                sock.close()
            except Exception:
                pass
            return None
        except socket.error:
            DoH_net_stats['conn_filtered'] += 1
            log_console('VOICE FILTERED' if is_voice else 'FILTERED', f'{host} ({ip}):{port}')
            _safe_send(client, b'HTTP/1.1 502 Bad Gateway\r\nProxy-agent: MyProxy/1.0\r\n\r\n')
            client.close()
            try:
                sock.close()
            except Exception:
                pass
            return ip if 'ip' in dir() else None
        except Exception as e:
            log_console('VOICE ERR' if is_voice else 'CONNECT ERR', repr(e))
            _safe_send(client, b'HTTP/1.1 502 Bad Gateway\r\nProxy-agent: MyProxy/1.0\r\n\r\n')
            client.close()
            try:
                sock.close()
            except Exception:
                pass
            return None


def _is_discord_domain(host):
    host_lower = host.lower()
    return any(d in host_lower for d in ('discord.com', 'discord.gg', 'discordapp.com', 'discordapp.net', 'discord.media'))

def _is_voice_port(port):
    return port in (443, 50000, 50001) or 50000 <= port <= 65535

def _is_discord_voice(host):
    host_lower = host.lower()
    return any(d in host_lower for d in ('voice', 'turn', 'stun', 'media', 'rtc', 'video'))


def _ping_tcp(ip, port=443, timeout=3):
    try:
        t0 = time.time()
        s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        s.settimeout(timeout)
        s.connect((ip, port))
        s.close()
        return round((time.time() - t0) * 1000, 1)
    except Exception:
        return None


def _feed_discord_ips(ips):
    with discord_lock:
        for ip in ips:
            if ip not in discord_ips and len(discord_ips) < CONFIG['discord_max_ips']:
                discord_ips[ip] = {'rtt': None, 'last_ping': 0, 'samples': []}
                log_console('DISCORD', f'added IP {ip} to ping pool')


def _best_discord_ip():
    with discord_lock:
        return discord_best_ip


def _ping_discord_loop():
    while True:
        time.sleep(CONFIG['discord_ping_interval'])
        with discord_lock:
            targets = list(discord_ips.keys())
        if not targets:
            continue
        best_rtt = float('inf')
        best_ip = None
        for ip in targets:
            rtt = _ping_tcp(ip, timeout=CONFIG['discord_ping_timeout'])
            with discord_lock:
                if ip in discord_ips:
                    if rtt is not None:
                        discord_ips[ip]['rtt'] = rtt
                        discord_ips[ip]['last_ping'] = time.time()
                        discord_ips[ip]['samples'].append(rtt)
                        if len(discord_ips[ip]['samples']) > 10:
                            discord_ips[ip]['samples'].pop(0)
                        avg_rtt = sum(discord_ips[ip]['samples']) / len(discord_ips[ip]['samples'])
                        if avg_rtt < best_rtt:
                            best_rtt = avg_rtt
                            best_ip = ip
                    else:
                        discord_ips[ip]['rtt'] = None
        if best_ip:
            with discord_lock:
                global discord_best_ip, discord_best_rtt
                discord_best_ip = best_ip
                discord_best_rtt = round(best_rtt, 1)
            log_console('DISCORD', f'best IP updated: {best_ip} (avg RTT={discord_best_rtt}ms)')


def _start_discord_pinger():
    t = threading.Thread(target=_ping_discord_loop, daemon=True)
    t.start()


def _safe_send(sock, data):
    try:
        sock.sendall(data)
    except Exception:
        pass


def _send_fragment(data, sock):
    length = len(data)
    if length <= 1:
        sock.sendall(data)
        return
    indices = sorted(random.sample(range(1, length - 1),
                                    min(CONFIG['num_fragment'] - 1, length - 2)))
    prev = 0
    for i in indices:
        sock.sendall(data[prev:i])
        prev = i
        time.sleep(CONFIG['fragment_sleep'])
    sock.sendall(data[prev:length])


def _build_stats():
    merged = {**DNS_cache, **offline_DNS}
    rev = {v: k for k, v in merged.items()}
    stats = {}
    for ip in IP_UL_traffic:
        up_kb = round(IP_UL_traffic[ip] / 1024.0, 3)
        down_kb = round(IP_DL_traffic[ip] / 1024.0, 3)
        host = rev.get(ip, '?')
        filtered = 'yes' if down_kb < 1.0 else '---'
        stats[ip] = f'UL={up_kb}KB DL={down_kb}KB filtered={filtered} host={host}'
    return stats


def _log_writer():
    path = os.path.join(BASE_DIR, 'DNS_IP_traffic_info.txt')
    with open(path, 'w') as f:
        while True:
            time.sleep(CONFIG['log_every_N_sec'])
            stats = _build_stats()
            lines = []
            lines.append(f'=== DoH Network Health ===')
            lines.append(f'Uptime: {now_iso()}')
            lines.append(f'Current DoH:  #{DoH_net_stats["current_doh_index"]} {DoH_servers[DoH_net_stats["current_doh_index"]]}')
            lines.append(f'Total Switches: {DoH_net_stats["total_switch_count"]}')
            lines.append(f'Queries: {DoH_net_stats["total_queries"]} OK={DoH_net_stats["successful_queries"]} FAIL={DoH_net_stats["failed_queries"]}')
            lines.append(f'Connections: {DoH_net_stats["conn_total"]} OK={DoH_net_stats["conn_success"]} FILTERED={DoH_net_stats["conn_filtered"]}')
            lines.append('')
            lines.append('--- Discord Best IP ---')
            with discord_lock:
                if discord_best_ip:
                    lines.append(f'  Best: {discord_best_ip} (avg RTT={discord_best_rtt}ms)')
                    lines.append(f'  Pool ({len(discord_ips)}):')
                    for dip, info in sorted(discord_ips.items()):
                        r = f'{info["rtt"]}ms' if info["rtt"] else 'N/A'
                        samples = len(info['samples'])
                        lines.append(f'    {dip:>15}  RTT={r:>7}  samples={samples}')
                else:
                    lines.append('  (no Discord IPs discovered yet)')
            lines.append('')
            lines.append('--- DoH Server Performance (last RTT) ---')
            for i, perf in DoH_net_stats['doh_perf'].items():
                rtt = perf['last_rtt']
                ok = perf['success_count']
                fail = perf['fail_count']
                rtt_str = f'{rtt}ms' if rtt else 'N/A'
                lines.append(f'  #{i:2d} RTT={rtt_str:>8} OK={ok:3d} FAIL={fail:2d} {DoH_servers[i]}')
            lines.append('')
            lines.append('--- DNS Cache ---')
            for domain, ip in sorted(DNS_cache.items()):
                lines.append(f'  {domain} -> {ip}')
            lines.append('')
            lines.append('--- Traffic Stats ---')
            for ip, line in sorted(stats.items()):
                lines.append(f'  {line}')

            f.seek(0)
            f.write('\n'.join(lines) + '\n')
            f.flush()
            f.truncate()


def _start_log_writer():
    t = threading.Thread(target=_log_writer, daemon=True)
    t.start()


if __name__ == '__main__':
    DoH_net_stats['start_time'] = now_iso()
    _start_log_writer()
    _start_discord_pinger()
    log_console('START', f'Listening on 127.0.0.1:{CONFIG["listen_PORT"]}')
    ThreadedServer('', CONFIG['listen_PORT']).listen()
