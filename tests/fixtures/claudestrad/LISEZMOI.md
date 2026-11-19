# Disquettes de test de CLAUDESTRAD (2026-11-19)

Une disquette par essai, avec ce qu'affiche CLAUDESTRAD (`claudestrad_crtc0.png`, `_crtc1.png`, `_crtc2.png` : 6128 avec
ce CRTC ; `_6128plus.png` pour les essais du Plus) et, dans `photos/`, les photos des vraies machines de l'utilisateur
(réduites à 1600 pixels, format AVIF : navigateurs récents, Windows 11 ; l'application Photos de Windows peut
demander l'extension gratuite « AV1 Video Extension »). Le nom de chaque photo commence par la machine quand elle est connue : `crtc1_sans_m4_…`,
`crtc2_m4_…` (carte M4 branchée : elle retarde l'interruption d'un top), etc.

Captures faites avec le moteur de la livraison du 2026-11-19 ; les essais qui dépendent du hasard (PPIRACE, nombre de
lectures fausses ; durées du lecteur de disquette) varient d'un passage à l'autre. `MESURES_A.DSK` contient la plupart des
essais sur une seule disquette (menu numéroté).

Remarques sur les captures : VSINT et VSINT1 plantent sur CRTC 2 (comme sur le vrai CRTC 2 de l'utilisateur) ; KUKU
n'affiche rien sur CRTC 1 (conforme au Compendium) ; les essais du lecteur de disquette (FDC…, SEEKJIT…, READYTST,
SPINDOWN, ROTATION) sont faits avec la disquette de l'essai dans le lecteur A ; NMICUR et PCSON avec la PlayCity.

| Essai | Disquette | Commande | Captures CLAUDESTRAD | Photos |
|---|---|---|---|---|
| INTPOS | `INTPOS.DSK` | `RUN"INTPOS` | crtc0, crtc1, crtc2 | 18 |
| INTPOS2 | `INTPOS2.DSK` | `RUN"INTPOS2` | crtc0, crtc1, crtc2 | 15 |
| INTPOS3 | `INTPOS3.DSK` | `RUN"INTPOS3` | crtc0, crtc1, crtc2 | 2 |
| INTREP | `INTREP.DSK` | `RUN"INTREP` | crtc0, crtc1, crtc2 | 5 |
| HALTPOS | `HALTPOS.DSK` | `RUN"HALTPOS` | crtc0, crtc1, crtc2 | 10 |
| HALTTST2 | `HALTTST2.DSK` | `RUN"HALTTST2`, puis une touche | crtc0, crtc1, crtc2 | 19 |
| HALTTEST | `HALTTEST.DSK` | `RUN"HALTTEST`, puis une touche | crtc0, crtc1, crtc2 | 5 |
| HALTPEND | `HALTPEND.DSK` | `RUN"HALTPEND` | crtc0, crtc1, crtc2 | — |
| INTLAT | `INTLAT.DSK` | `RUN"INTLAT` | crtc0, crtc1, crtc2 | 10 |
| INTLAT1 | `INTLAT1.DSK` | `RUN"INTLAT1` | crtc0, crtc1, crtc2 | — |
| VSINT | `VSINT.DSK` | `RUN"VSINT` | crtc0, crtc1, crtc2 | 8 |
| VSINT1 | `VSINT1.DSK` | `RUN"VSINT1` | crtc0, crtc1, crtc2 | — |
| PPIRACE | `PPIRACE.DSK` | `RUN"PPIRACE` | crtc0, crtc1, crtc2 | 8 |
| PALA14 | `PALA14.DSK` | `RUN"PALA14` | crtc0, crtc1, crtc2 | 9 |
| PALA14B | `PALA14B.DSK` | `RUN"PALA14B` | crtc0, crtc1, crtc2 | 8 |
| HSYNCPOS | `HSYNCPOS.DSK` | `RUN"HSYNCPOS`, puis une touche | crtc0, crtc1, crtc2 | 6 |
| INKBORD | `INKBORD.DSK` | `RUN"INKBORD`, puis une touche | crtc0, crtc1, crtc2 | 12 |
| INKMODE | `INKMODE.DSK` | `RUN"INKMODE` | crtc0, crtc1, crtc2 | 4 |
| INKPOINT | `INKPOINT.DSK` | `RUN"INKPOINT` | crtc0, crtc1, crtc2 | 5 |
| INKTEST | `INKTEST.DSK` | `RUN"INKTEST`, puis une touche | crtc0, crtc1, crtc2 | 2 |
| CRTCTEST | `CRTCTEST.DSK` | `RUN"CRTCTEST` | crtc0, crtc1, crtc2 | 10 |
| R0ZTEST | `R0ZTEST.DSK` | `RUN"R0ZTEST` | crtc0, crtc1, crtc2 | 6 |
| VSYNCTST | `VSYNCTST.DSK` | `RUN"VSYNCTST` | crtc0, crtc1, crtc2 | 1 |
| KUKU | `KUKU.DSK` | `RUN"KUKU` | crtc0, crtc1, crtc2 | 2 |
| Z80TYPE | `Z80TYPE.DSK` | `RUN"Z80TYPE` | crtc0, crtc1, crtc2 | 4 |
| CADRAGE | `CADRAGE.DSK` | `RUN"CADRAGE`, puis une touche | crtc0, crtc1, crtc2 | 3 |
| FDCTEST | `FDCTEST.DSK` | `RUN"FDCTEST` | crtc0, crtc1, crtc2 | — |
| FDCTEST2 | `FDCTEST2.DSK` | `RUN"FDCTEST2`, une touche après chaque test | crtc0, crtc1, crtc2 | — |
| FDCTEST3 | `FDCTEST3.DSK` | `RUN"FDCTEST3`, une touche après chaque test | crtc0, crtc1, crtc2 | — |
| SEEKJIT | `SEEKJIT.DSK` | `RUN"SEEKJIT` | crtc0, crtc1, crtc2 | 2 |
| SEEKJIT2 | `SEEKJIT.DSK` | `RUN"SEEKJIT2` | crtc0, crtc1, crtc2 | 2 |
| SEEKJIT3 | `SEEKJIT.DSK` | `RUN"SEEKJIT3` | crtc0, crtc1, crtc2 | 2 |
| SEEKJIT4 | `SEEKJIT.DSK` | `RUN"SEEKJIT4` | crtc0, crtc1, crtc2 | 2 |
| HSYNCW | `MESURES.DSK` | `RUN"HSYNCW`, puis une touche | crtc0, crtc1, crtc2 | 1 |
| FDCRES | `MESURES.DSK` | `RUN"FDCRES`, puis une touche | crtc0, crtc1, crtc2 | 2 |
| READYTST | `MESURES.DSK` | `RUN"READYTST`, puis une touche | crtc0, crtc1, crtc2 | 1 |
| SPINDOWN | `MESURES.DSK` | `RUN"SPINDOWN`, puis une touche | crtc0, crtc1, crtc2 | 1 |
| SPINDWN2 | `MESURES.DSK` | `RUN"SPINDWN2`, puis une touche | crtc0, crtc1, crtc2 | 1 |
| ROTATION | `MESURES.DSK` | `RUN"ROTATION`, puis une touche | crtc0, crtc1, crtc2 | 2 |
| AYLECT | `MESURES.DSK` | `RUN"AYLECT` | crtc0, crtc1, crtc2 | 1 |
| AYSEQ | `MESURES.DSK` | `RUN"AYSEQ` | crtc0, crtc1, crtc2 | 1 |
| AYTRAME | `MESURES.DSK` | `RUN"AYTRAME` | crtc0, crtc1, crtc2 | — |
| VHOLDTST | `MESURES.DSK` | `RUN"VHOLDTST` | crtc0, crtc1, crtc2 | — |
| NMICUR | `PLAYCITY_MESURE.DSK` | `RUN"NMICUR` | crtc0, crtc1, crtc2 | 1 |
| PCSON | `PCSON.DSK` | `RUN"PCSON` | crtc0, crtc1, crtc2 | 15 |
| PLUSDMA | `PLUSDMA.DSK` | `RUN"PLUSDMA`, puis une touche | 6128plus | 1 |
| PLUSDMA2 | `PLUSDMA2.DSK` | `RUN"PLUSDMA2`, puis une touche | 6128plus | 1 |
| PLUSIM2 | `PLUSIM2.DSK` | `RUN"PLUSIM2`, puis une touche | 6128plus | 1 |

Disquettes sans capture :

| Dossier | Disquette | Rôle | Photos |
|---|---|---|---|
| MESURES_A | `MESURES_A.DSK` | menu de tous les essais (RUN"MENU) | — |
| SERIE | `SERIE.DSK` | série CRTC 0 / CRTC 2 (les mêmes essais, voir chaque dossier) | — |
| Z80TEST | `z80full.dsk` | z80test de Patrik Rak, version complète (long) | 1 |
| Z80TEST | `z80ccf.dsk` | z80test, SCF/CCF seulement | — |
| ACID | `cpc.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `crtc1.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `crtc2.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `cpc_fdctest.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `test.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `asic1.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `asic2.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `ppi.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `psg.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
| ACID | `cpu.dsk` | suite acid (Kevin Thacker), voir tools/acid/LISEZMOI.md | — |
