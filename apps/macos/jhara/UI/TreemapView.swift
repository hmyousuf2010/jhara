// TreemapView.swift
// Squarified treemap rendered on SwiftUI Canvas.
// Color by safety tier. Hover tooltip. Click to select project.

import SwiftUI

// MARK: - Treemap layout

struct TreemapRect: Identifiable {
    let id:       UUID
    let artifact: Artifact
    var rect:     CGRect
}

/// Squarified treemap layout algorithm.
/// Reference: Bruls, Huizing, van Wijk — "Squarified Treemaps" (2000).
enum SquarifiedLayout {

    static func layout(
        items:  [(id: UUID, size: Int64)],
        in rect: CGRect
    ) -> [UUID: CGRect] {
        guard !items.isEmpty else { return [:] }

        let total  = items.reduce(0.0) { $0 + Double($1.size) }
        guard total > 0 else { return [:] }

        var result: [UUID: CGRect] = [:]
        var remaining = items.map { (id: $0.id, norm: Double($0.size) / total) }
        squarify(remaining: &remaining, row: [], rect: rect, result: &result)
        return result
    }

    private static func squarify(
        remaining: inout [(id: UUID, norm: Double)],
        row:        [(id: UUID, norm: Double)],
        rect:       CGRect,
        result:     inout [UUID: CGRect]
    ) {
        guard !remaining.isEmpty else {
            layoutRow(row, in: rect, result: &result)
            return
        }

        let w    = min(rect.width, rect.height)
        let next = remaining[0]

        if row.isEmpty || worstRatio(row + [next], width: w) <= worstRatio(row, width: w) {
            remaining.removeFirst()
            squarify(remaining: &remaining, row: row + [next], rect: rect, result: &result)
        } else {
            let usedRect = layoutRow(row, in: rect, result: &result)
            squarify(remaining: &remaining, row: [], rect: usedRect, result: &result)
        }
    }

    @discardableResult
    private static func layoutRow(
        _ row:    [(id: UUID, norm: Double)],
        in rect:  CGRect,
        result:   inout [UUID: CGRect]
    ) -> CGRect {
        guard !row.isEmpty else { return rect }
        let rowSum = row.reduce(0.0) { $0 + $1.norm }
        let w      = min(rect.width, rect.height)
        let h      = rowSum * (rect.width * rect.height) / w

        let isHorizontal = rect.width >= rect.height
        var offset: CGFloat = 0

        for item in row {
            let fraction = item.norm / rowSum
            let itemRect: CGRect
            if isHorizontal {
                itemRect = CGRect(
                    x: rect.minX + offset,
                    y: rect.minY,
                    width:  w * CGFloat(fraction),
                    height: h
                )
                offset += w * CGFloat(fraction)
            } else {
                itemRect = CGRect(
                    x: rect.minX,
                    y: rect.minY + offset,
                    width:  h,
                    height: w * CGFloat(fraction)
                )
                offset += w * CGFloat(fraction)
            }
            result[item.id] = itemRect.insetBy(dx: 1, dy: 1)
        }

        if isHorizontal {
            return CGRect(x: rect.minX, y: rect.minY + h,
                          width: rect.width, height: rect.height - h)
        } else {
            return CGRect(x: rect.minX + h, y: rect.minY,
                          width: rect.width - h, height: rect.height)
        }
    }

    private static func worstRatio(
        _ row: [(id: UUID, norm: Double)],
        width: CGFloat
    ) -> Double {
        guard !row.isEmpty else { return .infinity }
        let s = row.reduce(0.0) { $0 + $1.norm }
        let w = Double(width)
        let maxVal = row.max(by: { $0.norm < $1.norm })!.norm
        let minVal = row.min(by: { $0.norm < $1.norm })!.norm
        return max(w * w * maxVal / (s * s), s * s / (w * w * minVal))
    }
}

// MARK: - TreemapView

struct TreemapView: View {
    let projects:        [ProjectResult]
    var onSelect:        ((ProjectResult) -> Void)?

    @State private var rects:          [UUID: CGRect] = [:]
    @State private var hoveredArtifact: Artifact?
    @State private var tooltipPosition: CGPoint = .zero
    @State private var canvasSize:      CGSize  = .zero

    // Flat list of all artifacts for layout.
    private var allArtifacts: [Artifact] {
        projects.flatMap(\.artifacts)
    }

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .topLeading) {
                treemapCanvas(size: geo.size)
                if let artifact = hoveredArtifact {
                    TooltipView(artifact: artifact)
                        .position(clampedTooltip(at: tooltipPosition, in: geo.size))
                        .zIndex(10)
                        .transition(.opacity)
                        .animation(.easeIn(duration: 0.1), value: hoveredArtifact?.id)
                }
            }
            .onAppear  { rebuildLayout(size: geo.size) }
            .onChange(of: geo.size)   { _, s in rebuildLayout(size: s) }
            .onChange(of: allArtifacts.count) { _, _ in rebuildLayout(size: geo.size) }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Disk usage treemap")
    }

    // MARK: Canvas

    private func treemapCanvas(size: CGSize) -> some View {
        Canvas { ctx, _ in
            for artifact in allArtifacts {
                guard let rect = rects[artifact.id] else { continue }
                let color = tierColor(artifact.tier)
                ctx.fill(Path(rect), with: .color(color))
                // Label if rect is large enough
                if rect.width > 40 && rect.height > 20 {
                    var text = AttributedString(artifact.name)
                    text.font = .system(size: 9, weight: .medium)
                    text.foregroundColor = .white
                    ctx.draw(
                        Text(text),
                        in: rect.insetBy(dx: 3, dy: 3)
                    )
                }
            }
        }
        .frame(width: size.width, height: size.height)
        .onContinuousHover { phase in
            switch phase {
            case .active(let loc):
                tooltipPosition = loc
                hoveredArtifact = artifact(at: loc)
            case .ended:
                hoveredArtifact = nil
            }
        }
        .onTapGesture { loc in
            if let artifact = artifact(at: loc),
               let project  = projects.first(where: { $0.artifacts.contains(where: { $0.id == artifact.id }) }) {
                onSelect?(project)
            }
        }
        .accessibilityAddTraits(.isButton)
    }

    // MARK: Helpers

    private func rebuildLayout(size: CGSize) {
        canvasSize = size
        let items = allArtifacts.map { (id: $0.id, size: max($0.size, 1)) }
        rects = SquarifiedLayout.layout(
            items: items,
            in:    CGRect(origin: .zero, size: size)
        )
    }

    private func artifact(at point: CGPoint) -> Artifact? {
        for artifact in allArtifacts {
            if rects[artifact.id]?.contains(point) == true {
                return artifact
            }
        }
        return nil
    }

    private func tierColor(_ tier: SafetyTier) -> Color {
        switch tier {
        case .safe:    return Color(red: 0.20, green: 0.70, blue: 0.40).opacity(0.85)
        case .caution: return Color(red: 0.95, green: 0.65, blue: 0.10).opacity(0.85)
        case .risky:   return Color(red: 0.85, green: 0.25, blue: 0.25).opacity(0.85)
        }
    }

    private func clampedTooltip(at point: CGPoint, in size: CGSize) -> CGPoint {
        let tw: CGFloat = 200, th: CGFloat = 80
        let x = min(max(point.x + 12, tw / 2), size.width  - tw / 2)
        let y = min(max(point.y + 12, th / 2), size.height - th / 2)
        return CGPoint(x: x, y: y)
    }
}

// MARK: - TooltipView

struct TooltipView: View {
    let artifact: Artifact

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(artifact.name)
                .font(.caption.bold())
                .lineLimit(1)
            Text(ByteCountFormatter.string(fromByteCount: artifact.size, countStyle: .file))
                .font(.caption)
                .foregroundStyle(.secondary)
            Text(artifact.reason)
                .font(.caption2)
                .foregroundStyle(.secondary)
                .lineLimit(2)
            Text("Last activity: \(artifact.lastActivity.formatted(.relative(presentation: .named)))")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(8)
        .frame(width: 200, alignment: .leading)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 8))
        .shadow(radius: 4)
    }
}
