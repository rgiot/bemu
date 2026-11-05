; - Last Update : 22/04/2022
;
; On commence en premier lieu par tester si on est sur Amstrad Plus ou pas
;
ld bc,#f782 ; bc=#f782 ; Port B du PPi en entrée
ld e,c      ; e=#82 ; le test est basé sur le bug du ppi
out (c),c   ; de l'Amstrad Plus
dec b       ; b=#f6
ld c,#0f    ; bc=#F60f
out (c),c   ; on envoie une valeur bidon au port #f6 (#0f)
inc  b      ; b=#f7
ld a,e      ; a=#82
out (c),a   ; Port B du PPi en entrée
dec b       ; b=#f6
in a,(c)    ; a=val du port #f6
cp c        ; si la val du port #f6 est égale à #0f
jr z,crtc3  ; c'est un Amstrad Plus (Crtc 3)
;
; On est pas sur Amstrad Plus
; Les tests crtc 4 et 0 sont basés sur la lecture du poids fort
; du reg12 du crtc
;
ld e,#30 ; on positionne le poids fort de la vRam en #30 (#cxxx)
ld bc,#bc0c
out (c),c ; selection du r12
inc b
out (c),e ; envoie de la valeur au port 12 du crtc
;
; Est-ce un Amstrad Cpc Crtc 4 ?
; On lit le port #be
; Si le résultat obtenu est égal à la valeur de e (vRam)
; Alors c'est un Amstrad Cpc Crtc 4
;
ld b,#be ; detection crtc 4
in a,(c)
cp e ; e=poids fort vRam
jr z,crtc4
;
; Si ce n'est pas un Amstrad Cpc Crtc 4
; Est ce un Amstrad Cpc Crtc 0 ?
; Pour cela on utilise le port #bf du Crtc
; Si le résultat obtenu est égal à la valeur de e (vRam)
; alors c'est un Amsttrad Cpc Crtc 0
;
ld b,#bf ; detection crtc 0
in a,(c)
cp e ; e=poids fort vRam
jr z,crtc0
;
; Ce n'est pas un Amstrad Cpc Crtc 0
; Serait-ce par hasard un Amstrad Cpc Crtc 1 ?
;
ld bc,#bc1F ; on teste si c'est un crtc 1
out (c),c ; a sera différent de 0
;
ld b,#bf ; si a=0, ce sera un crtc 2 par élimination.
in a,(c)
jr z,crtc2
;
; Ce n'est pas un Amstrad Cpc Crtc 2
; c'est donc forcément par élimination, un Amstrad Cpc Crtc 1
;
jr crtc1
;
; it's a crtc 0
;
crtc0
crtc1
crtc2
crtc3
crtc4


