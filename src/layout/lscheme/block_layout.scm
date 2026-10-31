; Block layout algorithm — handles both block and inline layout in one recursive pass.
;
; Block ECS bindings (from setup_ecs_env):
;   (entity-block-context entity-id) -> list of (type entity-id) | #f
;   (entity-compute-dims entity-id container-width)
;       -> (decoration inline-size inline-offset) | #f
;   (entity-css-height entity-id parent-width) -> number | #f
;   (entity-intrinsic-size entity-id) -> (w h baseline) | #f
;   (entity-has-inline-context entity-id) -> #t | #f
;   (entity-position entity-id) -> (x y w h z page) | #f
;
; Absolutely positioned boxes (position: absolute):
;   (entity-inner-context entity-id) -> list of (type entity-id)
;       the flow inside of a box that is laid out on its own
;   (entity-absolute-decoration entity-id cb-width) -> decoration (margins are 0)
;   (entity-absolute-horizontal entity-id cb-width static-left)
;       -> (left width margin-left margin-right)
;   (entity-absolute-vertical entity-id cb-width cb-height static-top content-height)
;       -> (top height margin-top margin-bottom top-is-auto)
;   (begin-absolute-group! containing-block-entity-or-#f) / (end-absolute-group!)
;       everything that gets a position in between is placed on the page of the containing block
;   (set-group-containing-block! entity-or-#f)
;       changes the entity whose page the innermost open group is placed on
;   (translate-group! dy)
;       moves everything in the innermost open group down
;
; Inline ECS bindings:
;   (entity-inline-text-align entity-id) -> symbol | #f
;   (entity-text-indent entity-id line-width) -> number
;   (layout-inline-lines entity-id line-width indent)
;       -> list of (from to height baseline line-entity-id content-width)
;       Knuth-Plass line breaking is done in Rust; creates the line entities.
;
;   (entity-inline-blocks entity-id) -> list of entity ids
;       the inline-block boxes in the inline context of an entity
;   (finish-inline-blocks! entity-id)
;       tells the inline context the sizes of its inline-block boxes
;
; Inline-block boxes:
;   (entity-inline-block-dims entity-id available-width) -> (decoration content-width)
;   (begin-inline-block-group! entity-id) / (end-absolute-group!)
;       everything that gets a position in between is the contents of the box
;   (set-inline-block-size! entity-id width height baseline)
;
; Write bindings (inline during eval):
;   (set-position! entity x y w h z page)
;   (set-decoration! entity decoration)
;   (append-to-text-list! type entity)

; === Page cursor ===
; cursor = (vertical-pos collapse-margin)

(define (make-cursor) (list 0.0 #f))
(define (cursor-vpos c) (car c))
(define (cursor-cmargin c) (cadr c))

(define (cursor-add-content c amt)
  (if (= amt 0.0)
    c
    (list (+ (cursor-vpos c) amt) #f)))

(define (cursor-add-margin c margin)
  (if (= margin 0.0)
    c
    (let ((prev (cursor-cmargin c)))
      (if prev
        (let* ((new-m (max margin prev))
               (extra (- new-m prev)))
          (list (+ (cursor-vpos c) extra) new-m))
        (list (+ (cursor-vpos c) margin) margin)))))

; === Decoration accessors ===

(define (deco-margin d) (car d))
(define (deco-padding d) (cadr d))
(define (deco-border d) (caddr d))

(define (edge-top e)    (car e))
(define (edge-right e)  (cadr e))
(define (edge-bottom e) (caddr e))
(define (edge-left e)   (cadddr e))

(define (deco-margin-top d)    (edge-top    (deco-margin d)))
(define (deco-margin-bottom d) (edge-bottom (deco-margin d)))
(define (deco-border-top d)    (edge-top    (deco-border d)))
(define (deco-border-bottom d) (edge-bottom (deco-border d)))
(define (deco-padding-top d)   (edge-top    (deco-padding d)))
(define (deco-padding-bottom d)(edge-bottom (deco-padding d)))

; === Position accessors ===

(define (pos-x p) (car p))
(define (pos-y p) (cadr p))
(define (pos-w p) (caddr p))
(define (pos-h p) (cadddr p))
(define (pos-z p) (list-ref p 4))
(define (pos-page p) (list-ref p 5))

; Apply text alignment — returns (x adjusted-width)
(define (apply-text-align align lx lw content-w)
  (cond
    ((eq? align 'left)    (list lx content-w))
    ((eq? align 'right)   (list (+ lx (- lw content-w)) content-w))
    ((eq? align 'center)  (list (+ lx (/ (- lw content-w) 2.0)) content-w))
    ((eq? align 'justify) (list lx lw))
    (else                  (list lx content-w))))

; === Absolutely positioned boxes ===

; Distance between the z indices of boxes at different levels of nested absolutely positioned
; boxes. Absolutely positioned boxes are painted on top of the in-flow content.
(define abs-z-step 100000)

; Lays out an absolutely positioned box and the boxes that are positioned relative to it.
;   cb         (x y w h) padding box of the containing block
;   cb-entity  the entity establishing the containing block, #f for the initial containing block
;   static-pos (x y) margin edge of the box in its static position
;   z-base     z index offset of the box
;   anchor     the entity in the flow in front of the static position (or #f). A box that is
;              positioned by its static position goes on the page of that entity.
; https://www.w3.org/TR/CSS22/visudet.html#abs-non-replaced-width
(define (layout-absolute entity-id cb cb-entity static-pos z-base anchor)
  (let* ((cb-x (car cb))
         (cb-y (cadr cb))
         (cb-w (caddr cb))
         (cb-h (cadddr cb))
         (h    (entity-absolute-horizontal entity-id cb-w (- (car static-pos) cb-x)))
         (left (car h))
         (width (cadr h))
         (ml   (caddr h))
         (mr   (cadddr h))
         (deco0 (entity-absolute-decoration entity-id cb-w))
         (pl   (edge-left  (deco-padding deco0)))
         (pr   (edge-right (deco-padding deco0)))
         (pt   (deco-padding-top deco0))
         (pb   (deco-padding-bottom deco0))
         (bl   (edge-left (deco-border deco0)))
         (bt   (deco-border-top deco0))
         (z    (+ z-base 1))
         (content-x (+ cb-x left ml bl pl)))
    (begin-absolute-group! cb-entity)
    ; The content is laid out starting at y = 0 because the vertical position may depend on the
    ; height of the content. The whole group is moved to its final place afterwards.
    (set-position! entity-id content-x 0.0 width 0.0 z 0)
    (set-decoration! entity-id deco0)
    (let* ((flow (layout-flow (entity-inner-context entity-id) content-x width 0.0 z #f))
           (content-h (car flow))
           (pending (cadr flow))
           (v  (entity-absolute-vertical entity-id cb-w cb-h (- (cadr static-pos) cb-y) content-h))
           (top (car v))
           (height (cadr v))
           (mt (caddr v))
           (mb (cadddr v))
           (content-y (+ cb-y top mt bt pt))
           (deco (cons (list mt mr mb ml) (cdr deco0))))
      (when (and (not cb-entity) (list-ref v 4))
        (set-group-containing-block! anchor))
      (translate-group! content-y)
      (set-position! entity-id content-x content-y width height z 0)
      (set-decoration! entity-id deco)
      (end-absolute-group!)
      ; boxes inside of this box that are positioned relative to it
      (let ((nested-cb (list (- content-x pl) (- content-y pt) (+ width pl pr) (+ height pt pb))))
        (for-each
          (lambda (p)
            (layout-absolute (car p)
                             nested-cb
                             entity-id
                             (list (cadr p) (+ (caddr p) content-y))
                             (+ z-base abs-z-step)
                             #f))
          pending)))))

;; === Inline-block boxes ===

; Lays out an inline-block box on its own and tells the inline context it is part of how big the
; box is. The contents are laid out relative to the top left corner of the margin box of the
; box. They are moved to the place of the box on its line after the lines are laid out.
;   avail-w  the width of the line
;   z        z index of the box
; https://www.w3.org/TR/CSS22/visudet.html#inlineblock-width
(define (layout-inline-block entity-id avail-w z)
  (let* ((dims    (entity-inline-block-dims entity-id avail-w))
         (deco    (car dims))
         (width   (cadr dims))
         (margin  (deco-margin deco))
         (padding (deco-padding deco))
         (border  (deco-border deco))
         (ml (edge-left margin))   (mr (edge-right margin))
         (mt (edge-top margin))    (mb (edge-bottom margin))
         (pl (edge-left padding))  (pr (edge-right padding))
         (pt (edge-top padding))   (pb (edge-bottom padding))
         (bl (edge-left border))   (br (edge-right border))
         (bt (edge-top border))    (bb (edge-bottom border))
         (content-x (+ ml bl pl))
         (content-y (+ mt bt pt)))
    (begin-inline-block-group! entity-id)
    (set-position! entity-id content-x content-y width 0.0 z 0)
    (set-decoration! entity-id deco)
    (let* ((flow       (layout-flow (entity-inner-context entity-id) content-x width content-y z #f))
           (content-h  (- (car flow) content-y))
           (css-h      (entity-css-height entity-id avail-w))
           (height     (if css-h css-h content-h))
           (total-w    (+ ml bl pl width pr br mr))
           (total-h    (+ mt bt pt height pb bb mb))
           (last-line  (caddr flow)))
      (set-position! entity-id content-x content-y width height z 0)
      (end-absolute-group!)
      ; the baseline is the baseline of the last line, or the bottom margin edge without lines
      (set-inline-block-size! entity-id total-w total-h (if last-line last-line total-h)))))

; === Flow layout ===

; Lays out a list of block-level elements.
;   elems    list of (type entity-id)
;   base-x   left edge of the content box of the container the flow lives in
;   base-w   width of that content box
;   start-y  vertical position of the first element
;   z-base   z index of the container
;   icb      (x y w h) of the initial containing block: absolutely positioned boxes found in the
;            flow are placed relative to it right away. If #f their placement is postponed
;            because their containing block is not finished yet.
; Returns (end-y pending last-baseline) where pending are the postponed boxes as (entity static-x static-y)
; and last-baseline is the position of the baseline of the last line or #f.
(define (layout-flow elems base-x base-w start-y z-base icb)

  (define cursor (list start-y #f))
  (define container-stack '())
  (define pending '())
  ; the last entity that was placed in the flow
  (define last-entity #f)
  ; the position of the baseline of the last line, or #f if there were no lines
  (define last-baseline #f)

  (define (z-depth) (+ z-base 1 (length container-stack)))

  (define (parent-pos)
    (if (null? container-stack)
      (list base-x 0.0 base-w 0.0)
      (let* ((par-id (car (car container-stack)))
             (p (entity-position par-id)))
        (list (pos-x p) (pos-y p) (pos-w p) (pos-h p)))))

  (define (parent-width) (caddr (parent-pos)))
  (define (parent-x)     (car   (parent-pos)))

  ; --- ContainerStart ---
  (define (process-container-start entity-id)
    (let* ((par-w (parent-width))
           (par-x (parent-x))
           (dims  (entity-compute-dims entity-id par-w))
           (deco  (car dims))
           (isize (cadr dims))
           (ioff  (caddr dims)))
      (set! cursor (cursor-add-margin  cursor (deco-margin-top  deco)))
      (set! cursor (cursor-add-content cursor (deco-border-top  deco)))
      (set! cursor (cursor-add-content cursor (deco-padding-top deco)))
      (set-position! entity-id
                     (+ par-x ioff) (cursor-vpos cursor)
                     isize 0.0 (z-depth) 0)
      (set-decoration! entity-id deco)
      (set! container-stack
            (cons (list entity-id (cursor-vpos cursor) deco par-w)
                  container-stack))
      (set! last-entity entity-id)
      (append-to-text-list! 'container-start entity-id)))

  ; --- ContainerEnd ---
  (define (process-container-end entity-id)
    (let* ((frame     (car container-stack))
           (start-y   (cadr  frame))
           (deco      (caddr frame))
           (par-w     (cadddr frame))
           (pos       (entity-position entity-id))
           (pos-y-val (pos-y pos))
           (content-h (- (cursor-vpos cursor) pos-y-val))
           (css-h     (entity-css-height entity-id par-w)))
      (set! container-stack (cdr container-stack))
      (let ((height (if css-h css-h content-h)))
        (when (not (= height content-h))
          (set! cursor (cursor-add-content cursor (- height content-h))))
        (set-position! entity-id
                       (pos-x pos) pos-y-val (pos-w pos) height
                       (pos-z pos) (pos-page pos))
        (set! cursor (cursor-add-content cursor (deco-border-bottom  deco)))
        (set! cursor (cursor-add-content cursor (deco-padding-bottom deco)))
        (set! cursor (cursor-add-margin  cursor (deco-margin-bottom  deco)))
        (set! last-entity entity-id)
        (append-to-text-list! 'container-end entity-id))))

  ; --- Leaf ---
  (define (process-leaf entity-id)
    (if (entity-has-inline-context entity-id)
      ; Inline content: run Knuth-Plass line breaking
      (let* ((pos    (entity-position entity-id))
             (lx     (pos-x pos))
             (lw     (pos-w pos))
             (indent (entity-text-indent entity-id lw))
             (align  (entity-inline-text-align entity-id)))
        (define (layout-lines lines line-num)
          (when (pair? lines)
            (let* ((line     (car lines))
                   (height   (caddr line))
                   (line-ent (list-ref line 4))
                   (adv-w    (list-ref line 5))
                   (x0       (if (= line-num 0) (+ lx indent) lx))
                   (w0       (if (= line-num 0) (- lw indent) lw))
                   (xw       (apply-text-align align x0 w0 adv-w))
                   (x        (car  xw))
                   (w        (cadr xw)))
              (set-position! line-ent x (cursor-vpos cursor) w height (z-depth) 0)
              (set! last-baseline (+ (cursor-vpos cursor) (cadddr line)))
              (set! last-entity line-ent)
              (append-to-text-list! 'leaf line-ent)
              (set! cursor (cursor-add-content cursor height))
              (layout-lines (cdr lines) (+ line-num 1)))))
        ; the inline-block boxes have to be laid out first: their sizes decide where lines break
        (let ((boxes (entity-inline-blocks entity-id)))
          (when (pair? boxes)
            (for-each (lambda (b) (layout-inline-block b lw (z-depth))) boxes)
            (finish-inline-blocks! entity-id)))
        (layout-lines (layout-inline-lines entity-id lw indent) 0))
      ; Non-inline leaf (replaced element with intrinsic size)
      (let ((is (entity-intrinsic-size entity-id)))
        (when is
          (set! cursor (cursor-add-content cursor (cadr is)))
          (set! last-entity entity-id)
          (append-to-text-list! 'leaf entity-id)))))

  ; --- Absolutely positioned box: this is where it would have been in the flow ---
  (define (process-absolute entity-id)
    (let ((static-x (parent-x))
          (static-y (cursor-vpos cursor)))
      (if icb
        (layout-absolute entity-id icb #f (list static-x static-y) (+ z-base abs-z-step) last-entity)
        (set! pending (cons (list entity-id static-x static-y) pending)))))

  ; --- Main loop ---
  (define (process-elements elems)
    (when (pair? elems)
      (let* ((elem   (car elems))
             (type   (car elem))
             (ent-id (cadr elem)))
        (cond
          ((eq? type 'container-start) (process-container-start ent-id))
          ((eq? type 'container-end)   (process-container-end   ent-id))
          ((eq? type 'leaf)            (process-leaf             ent-id))
          ((eq? type 'absolute)        (process-absolute         ent-id)))
        (process-elements (cdr elems)))))

  (process-elements elems)
  (list (cursor-vpos cursor) (reverse pending) last-baseline))

; === Main entry point ===

(define (run-block-layout root-entity-id page-width page-height
                          margin-top margin-right margin-bottom margin-left)
  (let* ((width (- page-width margin-left margin-right))
         ; the initial containing block is the area of the first page
         (icb   (list margin-left 0.0 width (max 0.0 (- page-height margin-top margin-bottom)))))
    (layout-flow (entity-block-context root-entity-id) margin-left width 0.0 0 icb)
    #t))
