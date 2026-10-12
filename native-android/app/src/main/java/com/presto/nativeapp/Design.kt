package com.presto.nativeapp

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import java.time.YearMonth
import java.time.format.DateTimeFormatter

internal val Ink = Color(0xff223b35)
internal val Muted = Color(0xff606e66)
internal val SoftAccent = Color(0xffe1eee6)

internal fun monthLabel(month: String): String =
    runCatching { YearMonth.parse(month).format(DateTimeFormatter.ofPattern("MMMM yyyy")) }
        .getOrDefault(month)

/** Decorative icons; the containing labeled control supplies accessibility semantics. */
@Composable
internal fun PrestoIcon(
    kind: String,
    modifier: Modifier = Modifier,
    color: Color = LocalContentColor.current,
) {
    Canvas(modifier.size(24.dp)) {
        val unit = size.width / 24f
        fun line(x: Float, y: Float, xx: Float, yy: Float) =
            drawLine(
                color,
                Offset(x * unit, y * unit),
                Offset(xx * unit, yy * unit),
                1.8f * unit,
                StrokeCap.Round,
            )
        when (kind) {
            "add" -> {
                line(12f, 5f, 12f, 19f)
                line(5f, 12f, 19f, 12f)
            }
            "recent" -> {
                drawCircle(
                    color,
                    8f * unit,
                    Offset(12f * unit, 12f * unit),
                    style = Stroke(1.8f * unit),
                )
                line(12f, 7f, 12f, 12f)
                line(12f, 12f, 16f, 14f)
            }
            "budget" -> {
                drawRoundRect(
                    color,
                    Offset(4f * unit, 4f * unit),
                    Size(16f * unit, 16f * unit),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(3f * unit),
                    style = Stroke(1.8f * unit),
                )
                line(8f, 16f, 8f, 12f)
                line(12f, 16f, 12f, 8f)
                line(16f, 16f, 16f, 10f)
            }
            "accounts" -> {
                drawRoundRect(
                    color,
                    Offset(3f * unit, 5f * unit),
                    Size(18f * unit, 14f * unit),
                    cornerRadius = androidx.compose.ui.geometry.CornerRadius(3f * unit),
                    style = Stroke(1.8f * unit),
                )
                line(3f, 10f, 21f, 10f)
                line(7f, 15f, 11f, 15f)
            }
            "setup" -> {
                for (y in listOf(6f, 12f, 18f)) line(4f, y, 20f, y)
                for ((x, y) in listOf(9f to 6f, 15f to 12f, 9f to 18f)) {
                    drawCircle(Background, 2.5f * unit, Offset(x * unit, y * unit))
                    drawCircle(
                        color,
                        2.5f * unit,
                        Offset(x * unit, y * unit),
                        style = Stroke(1.8f * unit),
                    )
                }
            }
            "search" -> {
                drawCircle(
                    color,
                    6f * unit,
                    Offset(10f * unit, 10f * unit),
                    style = Stroke(1.8f * unit),
                )
                line(15f, 15f, 21f, 21f)
            }
            "next" -> {
                line(9f, 6f, 15f, 12f)
                line(15f, 12f, 9f, 18f)
            }
            "income" -> {
                line(7f, 17f, 17f, 7f)
                line(10f, 7f, 17f, 7f)
                line(17f, 7f, 17f, 14f)
            }
            "expense" -> {
                line(7f, 7f, 17f, 17f)
                line(10f, 17f, 17f, 17f)
                line(17f, 10f, 17f, 17f)
            }
            else -> {
                line(4f, 8f, 20f, 8f)
                line(16f, 4f, 20f, 8f)
                line(4f, 16f, 20f, 16f)
                line(4f, 16f, 8f, 20f)
            }
        }
    }
}

@Composable
internal fun StatusBadge(text: String, color: Color = Primary) {
    Text(
        text,
        color = color,
        style = MaterialTheme.typography.labelMedium,
        modifier =
            Modifier.background(color.copy(alpha = .09f), RoundedCornerShape(8.dp))
                .padding(horizontal = 8.dp, vertical = 4.dp),
    )
}

internal val ExpenseColor = Color(0xffa34a40)
internal val TransferColor = Color(0xff426a9a)
internal val BudgetMoveColor = Color(0xff79568b)
internal val PendingColor = Color(0xff8c620d)

internal fun transactionColor(kind: String): Color =
    when (kind) {
        "expense" -> ExpenseColor
        "income" -> Positive
        "transfer" -> TransferColor
        "reallocate" -> BudgetMoveColor
        else -> Ink
    }

@Composable
internal fun CompactSearch(
    value: String,
    change: (String) -> Unit,
    tag: String,
    modifier: Modifier = Modifier,
    placeholder: String = "Search",
) {
    OutlinedTextField(
        value = value,
        onValueChange = change,
        singleLine = true,
        placeholder = {
            Text(
                placeholder,
                maxLines = 1,
                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
            )
        },
        leadingIcon = { PrestoIcon("search", color = Muted) },
        textStyle = MaterialTheme.typography.bodyMedium,
        shape = RoundedCornerShape(12.dp),
        modifier =
            modifier.fillMaxWidth().testTag(tag).semantics { contentDescription = placeholder },
    )
}
